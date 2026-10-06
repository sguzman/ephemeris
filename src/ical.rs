use std::{collections::HashSet, fmt};

use chrono::{
    DateTime, Duration, LocalResult, NaiveDate, NaiveDateTime, Offset, TimeZone, Timelike, Utc,
};
use chrono_tz::Tz;

use crate::domain::{
    RecurrenceError, RecurrenceFrequency, RecurrenceOrdinalWeekday, RecurrenceRule,
    RecurrenceWeekday, TimeSpec,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IcalRecurrenceError {
    EmptyRule,
    MissingFrequency,
    DuplicatePart(String),
    UnknownPart(String),
    InvalidPart { name: String, value: String },
    CountUntilConflict,
    UnsupportedUntilDateTime(String),
    InvalidProperty(String),
    WrongProperty {
        expected: &'static str,
        actual: String,
    },
    DuplicateParameter(String),
    UnsupportedParameter(String),
    UnsupportedValueType(String),
    UnsupportedPeriod,
    InvalidTimezone(String),
    TemporalKindMismatch {
        property: &'static str,
        expected: &'static str,
        actual: &'static str,
    },
    TimezoneMismatch {
        expected: Option<String>,
        actual: String,
    },
    InvalidPropertyValue {
        property: &'static str,
        value: String,
    },
    FractionalSecondUnsupported {
        property: &'static str,
        value: String,
    },
    ExceptionShapeMismatch {
        property: &'static str,
    },
    EmptyPropertyValues(&'static str),
    UnsupportedBaseKind(&'static str),
    ArithmeticOverflow,
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
            Self::InvalidProperty(value) => write!(formatter, "invalid iCalendar property {value}"),
            Self::WrongProperty { expected, actual } => {
                write!(formatter, "expected {expected} property, got {actual}")
            }
            Self::DuplicateParameter(name) => {
                write!(formatter, "duplicate iCalendar property parameter {name}")
            }
            Self::UnsupportedParameter(name) => {
                write!(formatter, "unsupported iCalendar property parameter {name}")
            }
            Self::UnsupportedValueType(value) => {
                write!(formatter, "unsupported iCalendar recurrence value type {value}")
            }
            Self::UnsupportedPeriod => formatter.write_str(
                "RDATE VALUE=PERIOD is not supported until occurrence-specific periods are canonical",
            ),
            Self::InvalidTimezone(value) => {
                write!(formatter, "unsupported or invalid iCalendar TZID {value}")
            }
            Self::TemporalKindMismatch {
                property,
                expected,
                actual,
            } => write!(
                formatter,
                "{property} value kind {actual} does not match recurrence base kind {expected}"
            ),
            Self::TimezoneMismatch { expected, actual } => write!(
                formatter,
                "TZID {actual} does not match recurrence source timezone {}",
                expected.as_deref().unwrap_or("<none>")
            ),
            Self::InvalidPropertyValue { property, value } => {
                write!(formatter, "invalid {property} value {value}")
            }
            Self::FractionalSecondUnsupported { property, value } => write!(
                formatter,
                "{property} value {value} has fractional seconds that RFC 5545 DATE-TIME cannot preserve"
            ),
            Self::ExceptionShapeMismatch { property } => write!(
                formatter,
                "{property} occurrence does not preserve the recurrence base kind, duration, and source-timezone shape"
            ),
            Self::EmptyPropertyValues(property) => {
                write!(formatter, "{property} requires at least one value")
            }
            Self::UnsupportedBaseKind(kind) => {
                write!(formatter, "iCalendar recurrence dates are unsupported for {kind} precision")
            }
            Self::ArithmeticOverflow => formatter.write_str("iCalendar recurrence-date arithmetic overflow"),
            Self::Domain(error) => {
                write!(formatter, "invalid RRULE recurrence definition: {error}")
            }
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
        let (name, raw_value) =
            part.split_once('=')
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
                frequency = Some(RecurrenceFrequency::parse(raw_value).ok_or_else(|| {
                    IcalRecurrenceError::InvalidPart {
                        name,
                        value: raw_value.to_string(),
                    }
                })?);
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
        values.extend(
            rule.by_month_weekday
                .iter()
                .map(|selector| format!("{}{}", selector.ordinal, weekday_code(selector.weekday))),
        );
        parts.push(format!("BYDAY={}", values.join(",")));
    }

    push_list(&mut parts, "BYHOUR", &rule.by_hour);
    push_list(&mut parts, "BYMINUTE", &rule.by_minute);
    push_list(&mut parts, "BYSECOND", &rule.by_second);
    push_list(&mut parts, "BYSETPOS", &rule.by_set_pos);

    Ok(parts.join(";"))
}


#[derive(Debug)]
struct ParsedDateProperty<'a> {
    tzid: Option<String>,
    value_type: Option<String>,
    values: Vec<&'a str>,
}

pub fn parse_rdate_property(
    raw: &str,
    base: &TimeSpec,
) -> Result<Vec<TimeSpec>, IcalRecurrenceError> {
    parse_recurrence_date_property(raw, "RDATE", base, true)
}

pub fn parse_exdate_property(
    raw: &str,
    base: &TimeSpec,
) -> Result<Vec<TimeSpec>, IcalRecurrenceError> {
    parse_recurrence_date_property(raw, "EXDATE", base, false)
}

pub fn format_rdate_property(
    values: &[TimeSpec],
    base: &TimeSpec,
) -> Result<String, IcalRecurrenceError> {
    format_recurrence_date_property("RDATE", values, base)
}

pub fn format_exdate_property(
    values: &[TimeSpec],
    base: &TimeSpec,
) -> Result<String, IcalRecurrenceError> {
    format_recurrence_date_property("EXDATE", values, base)
}

fn parse_recurrence_date_property(
    raw: &str,
    expected_name: &'static str,
    base: &TimeSpec,
    allow_period: bool,
) -> Result<Vec<TimeSpec>, IcalRecurrenceError> {
    let property = parse_date_property(raw, expected_name)?;
    let value_type = property.value_type.as_deref().unwrap_or("DATE-TIME");
    if !matches!(value_type, "DATE" | "DATE-TIME" | "PERIOD") {
        return Err(IcalRecurrenceError::UnsupportedValueType(
            value_type.to_string(),
        ));
    }
    if value_type == "PERIOD" {
        if allow_period {
            return Err(IcalRecurrenceError::UnsupportedPeriod);
        }
        return Err(IcalRecurrenceError::UnsupportedValueType(
            value_type.to_string(),
        ));
    }

    let mut parsed = Vec::with_capacity(property.values.len());
    for raw_value in property.values {
        let value = parse_recurrence_date_value(
            expected_name,
            raw_value,
            value_type,
            property.tzid.as_deref(),
            base,
        )?;
        if !parsed.contains(&value) {
            parsed.push(value);
        }
    }
    Ok(parsed)
}

fn parse_date_property<'a>(
    raw: &'a str,
    expected_name: &'static str,
) -> Result<ParsedDateProperty<'a>, IcalRecurrenceError> {
    let raw = raw.trim();
    let (head, raw_values) = raw
        .split_once(':')
        .ok_or_else(|| IcalRecurrenceError::InvalidProperty(raw.to_string()))?;
    if raw_values.trim().is_empty() {
        return Err(IcalRecurrenceError::EmptyPropertyValues(expected_name));
    }

    let mut head_parts = head.split(';');
    let actual_name = head_parts
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_uppercase();
    if actual_name != expected_name {
        return Err(IcalRecurrenceError::WrongProperty {
            expected: expected_name,
            actual: actual_name,
        });
    }

    let mut seen = HashSet::new();
    let mut tzid = None;
    let mut value_type = None;
    for raw_parameter in head_parts {
        let (raw_name, raw_value) = raw_parameter
            .split_once('=')
            .ok_or_else(|| IcalRecurrenceError::InvalidProperty(raw.to_string()))?;
        let name = raw_name.trim().to_ascii_uppercase();
        if !seen.insert(name.clone()) {
            return Err(IcalRecurrenceError::DuplicateParameter(name));
        }
        let value = unquote_parameter_value(raw_value.trim());
        if value.is_empty() {
            return Err(IcalRecurrenceError::InvalidProperty(raw.to_string()));
        }
        match name.as_str() {
            "VALUE" => value_type = Some(value.to_ascii_uppercase()),
            "TZID" => tzid = Some(value.to_string()),
            _ => return Err(IcalRecurrenceError::UnsupportedParameter(name)),
        }
    }

    let values = raw_values.split(',').map(str::trim).collect::<Vec<_>>();
    if values.iter().any(|value| value.is_empty()) {
        return Err(IcalRecurrenceError::InvalidProperty(raw.to_string()));
    }

    Ok(ParsedDateProperty {
        tzid,
        value_type,
        values,
    })
}

fn unquote_parameter_value(raw: &str) -> &str {
    raw.strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(raw)
}

fn parse_recurrence_date_value(
    property: &'static str,
    raw: &str,
    value_type: &str,
    tzid: Option<&str>,
    base: &TimeSpec,
) -> Result<TimeSpec, IcalRecurrenceError> {
    match base {
        TimeSpec::DateOnly { .. } | TimeSpec::AllDay { .. } => {
            if value_type != "DATE" {
                return Err(IcalRecurrenceError::TemporalKindMismatch {
                    property,
                    expected: base.kind_name(),
                    actual: value_type_kind(value_type),
                });
            }
            if tzid.is_some() {
                return Err(IcalRecurrenceError::UnsupportedParameter(
                    "TZID".to_string(),
                ));
            }
            let start = NaiveDate::parse_from_str(raw, "%Y%m%d").map_err(|_| {
                IcalRecurrenceError::InvalidPropertyValue {
                    property,
                    value: raw.to_string(),
                }
            })?;
            shift_exception_date(base, start, property)
        }
        TimeSpec::Floating { .. } => {
            if value_type != "DATE-TIME" || tzid.is_some() || raw.ends_with('Z') {
                return Err(IcalRecurrenceError::TemporalKindMismatch {
                    property,
                    expected: base.kind_name(),
                    actual: value_type_kind(value_type),
                });
            }
            let start = parse_basic_datetime(property, raw)?;
            shift_exception_floating(base, start, property)
        }
        TimeSpec::Instant {
            source_timezone, ..
        } => {
            if value_type != "DATE-TIME" {
                return Err(IcalRecurrenceError::TemporalKindMismatch {
                    property,
                    expected: base.kind_name(),
                    actual: value_type_kind(value_type),
                });
            }

            let start_utc = if let Some(raw_tzid) = tzid {
                if raw.ends_with('Z') {
                    return Err(IcalRecurrenceError::InvalidPropertyValue {
                        property,
                        value: raw.to_string(),
                    });
                }
                if source_timezone.as_deref() != Some(raw_tzid) {
                    return Err(IcalRecurrenceError::TimezoneMismatch {
                        expected: source_timezone.clone(),
                        actual: raw_tzid.to_string(),
                    });
                }
                let timezone = raw_tzid
                    .parse::<Tz>()
                    .map_err(|_| IcalRecurrenceError::InvalidTimezone(raw_tzid.to_string()))?;
                let local = parse_basic_datetime(property, raw)?;
                resolve_ical_local_datetime(property, timezone, local)?
            } else {
                let utc_raw = raw.strip_suffix('Z').ok_or_else(|| {
                    IcalRecurrenceError::TemporalKindMismatch {
                        property,
                        expected: base.kind_name(),
                        actual: "floating date-time",
                    }
                })?;
                let utc = parse_basic_datetime(property, utc_raw)?;
                DateTime::<Utc>::from_naive_utc_and_offset(utc, Utc)
            };
            shift_exception_instant(base, start_utc, property)
        }
        TimeSpec::Month { .. } | TimeSpec::Year { .. } | TimeSpec::Unknown { .. } => {
            Err(IcalRecurrenceError::UnsupportedBaseKind(base.kind_name()))
        }
    }
}

const fn value_type_kind(value_type: &str) -> &'static str {
    match value_type {
        "DATE" => "date",
        "DATE-TIME" => "date-time",
        "PERIOD" => "period",
        _ => "unknown",
    }
}

fn parse_basic_datetime(
    property: &'static str,
    raw: &str,
) -> Result<NaiveDateTime, IcalRecurrenceError> {
    NaiveDateTime::parse_from_str(raw, "%Y%m%dT%H%M%S").map_err(|_| {
        IcalRecurrenceError::InvalidPropertyValue {
            property,
            value: raw.to_string(),
        }
    })
}

fn resolve_ical_local_datetime(
    property: &'static str,
    timezone: Tz,
    local: NaiveDateTime,
) -> Result<DateTime<Utc>, IcalRecurrenceError> {
    match timezone.from_local_datetime(&local) {
        LocalResult::Single(value) => Ok(value.with_timezone(&Utc)),
        LocalResult::Ambiguous(first, second) => {
            let first = first.with_timezone(&Utc);
            let second = second.with_timezone(&Utc);
            Ok(if first <= second { first } else { second })
        }
        LocalResult::None => {
            let mut probe = local;
            for _ in 0..=48 {
                probe = probe
                    .checked_sub_signed(Duration::hours(1))
                    .ok_or(IcalRecurrenceError::ArithmeticOverflow)?;
                let offset_seconds = match timezone.from_local_datetime(&probe) {
                    LocalResult::Single(value) => Some(value.offset().fix().local_minus_utc()),
                    LocalResult::Ambiguous(first, second) => {
                        let first_utc = first.with_timezone(&Utc);
                        let second_utc = second.with_timezone(&Utc);
                        let value = if first_utc <= second_utc { first } else { second };
                        Some(value.offset().fix().local_minus_utc())
                    }
                    LocalResult::None => None,
                };
                if let Some(offset_seconds) = offset_seconds {
                    let utc = local
                        .checked_sub_signed(Duration::seconds(i64::from(offset_seconds)))
                        .ok_or(IcalRecurrenceError::ArithmeticOverflow)?;
                    return Ok(DateTime::<Utc>::from_naive_utc_and_offset(utc, Utc));
                }
            }
            Err(IcalRecurrenceError::InvalidPropertyValue {
                property,
                value: local.to_string(),
            })
        }
    }
}

fn shift_exception_date(
    base: &TimeSpec,
    start: NaiveDate,
    property: &'static str,
) -> Result<TimeSpec, IcalRecurrenceError> {
    match base {
        TimeSpec::DateOnly {
            start: base_start,
            end_exclusive,
        } => Ok(TimeSpec::DateOnly {
            start,
            end_exclusive: shifted_date_end(*base_start, *end_exclusive, start)?,
        }),
        TimeSpec::AllDay {
            start: base_start,
            end_exclusive,
        } => Ok(TimeSpec::AllDay {
            start,
            end_exclusive: shifted_date_end(*base_start, *end_exclusive, start)?,
        }),
        _ => Err(IcalRecurrenceError::ExceptionShapeMismatch { property }),
    }
}

fn shifted_date_end(
    base_start: NaiveDate,
    base_end: Option<NaiveDate>,
    start: NaiveDate,
) -> Result<Option<NaiveDate>, IcalRecurrenceError> {
    base_end
        .map(|end| {
            start
                .checked_add_signed(end - base_start)
                .ok_or(IcalRecurrenceError::ArithmeticOverflow)
        })
        .transpose()
}

fn shift_exception_floating(
    base: &TimeSpec,
    start: NaiveDateTime,
    property: &'static str,
) -> Result<TimeSpec, IcalRecurrenceError> {
    let TimeSpec::Floating {
        start: base_start,
        end,
        source_timezone,
    } = base
    else {
        return Err(IcalRecurrenceError::ExceptionShapeMismatch { property });
    };
    let shifted_end = end
        .map(|value| {
            start
                .checked_add_signed(value - *base_start)
                .ok_or(IcalRecurrenceError::ArithmeticOverflow)
        })
        .transpose()?;
    Ok(TimeSpec::Floating {
        start,
        end: shifted_end,
        source_timezone: source_timezone.clone(),
    })
}

fn shift_exception_instant(
    base: &TimeSpec,
    start_utc: DateTime<Utc>,
    property: &'static str,
) -> Result<TimeSpec, IcalRecurrenceError> {
    let TimeSpec::Instant {
        start_utc: base_start,
        end_utc,
        source_timezone,
    } = base
    else {
        return Err(IcalRecurrenceError::ExceptionShapeMismatch { property });
    };
    let shifted_end = end_utc
        .map(|value| {
            start_utc
                .checked_add_signed(value - *base_start)
                .ok_or(IcalRecurrenceError::ArithmeticOverflow)
        })
        .transpose()?;
    Ok(TimeSpec::Instant {
        start_utc,
        end_utc: shifted_end,
        source_timezone: source_timezone.clone(),
    })
}

fn format_recurrence_date_property(
    property: &'static str,
    values: &[TimeSpec],
    base: &TimeSpec,
) -> Result<String, IcalRecurrenceError> {
    if values.is_empty() {
        return Err(IcalRecurrenceError::EmptyPropertyValues(property));
    }
    ensure_supported_exception_base(base)?;

    let mut encoded = Vec::with_capacity(values.len());
    for value in values {
        ensure_exception_shape(property, value, base)?;
        encoded.push(format_recurrence_date_value(property, value)?);
    }

    let header = match base {
        TimeSpec::DateOnly { .. } | TimeSpec::AllDay { .. } => {
            format!("{property};VALUE=DATE")
        }
        TimeSpec::Floating { .. } | TimeSpec::Instant { .. } => property.to_string(),
        TimeSpec::Month { .. } | TimeSpec::Year { .. } | TimeSpec::Unknown { .. } => {
            return Err(IcalRecurrenceError::UnsupportedBaseKind(base.kind_name()));
        }
    };
    Ok(format!("{header}:{}", encoded.join(",")))
}

fn ensure_supported_exception_base(base: &TimeSpec) -> Result<(), IcalRecurrenceError> {
    match base {
        TimeSpec::DateOnly { .. }
        | TimeSpec::AllDay { .. }
        | TimeSpec::Floating { .. }
        | TimeSpec::Instant { .. } => Ok(()),
        TimeSpec::Month { .. } | TimeSpec::Year { .. } | TimeSpec::Unknown { .. } => {
            Err(IcalRecurrenceError::UnsupportedBaseKind(base.kind_name()))
        }
    }
}

fn ensure_exception_shape(
    property: &'static str,
    value: &TimeSpec,
    base: &TimeSpec,
) -> Result<(), IcalRecurrenceError> {
    let same = match (base, value) {
        (
            TimeSpec::DateOnly {
                start: base_start,
                end_exclusive: base_end,
            },
            TimeSpec::DateOnly {
                start,
                end_exclusive,
            },
        )
        | (
            TimeSpec::AllDay {
                start: base_start,
                end_exclusive: base_end,
            },
            TimeSpec::AllDay {
                start,
                end_exclusive,
            },
        ) => optional_duration(*base_start, *base_end)
            == optional_duration(*start, *end_exclusive),
        (
            TimeSpec::Floating {
                start: base_start,
                end: base_end,
                source_timezone: base_timezone,
            },
            TimeSpec::Floating {
                start,
                end,
                source_timezone,
            },
        ) => {
            base_timezone == source_timezone
                && optional_duration(*base_start, *base_end) == optional_duration(*start, *end)
        }
        (
            TimeSpec::Instant {
                start_utc: base_start,
                end_utc: base_end,
                source_timezone: base_timezone,
            },
            TimeSpec::Instant {
                start_utc,
                end_utc,
                source_timezone,
            },
        ) => {
            base_timezone == source_timezone
                && optional_duration(*base_start, *base_end)
                    == optional_duration(*start_utc, *end_utc)
        }
        _ => false,
    };
    if same {
        Ok(())
    } else {
        Err(IcalRecurrenceError::ExceptionShapeMismatch { property })
    }
}

fn optional_duration<T>(start: T, end: Option<T>) -> Option<Duration>
where
    T: Copy + std::ops::Sub<T, Output = Duration>,
{
    end.map(|value| value - start)
}

fn format_recurrence_date_value(
    property: &'static str,
    value: &TimeSpec,
) -> Result<String, IcalRecurrenceError> {
    match value {
        TimeSpec::DateOnly { start, .. } | TimeSpec::AllDay { start, .. } => {
            Ok(start.format("%Y%m%d").to_string())
        }
        TimeSpec::Floating { start, .. } => {
            reject_fractional_seconds(property, *start)?;
            Ok(start.format("%Y%m%dT%H%M%S").to_string())
        }
        TimeSpec::Instant { start_utc, .. } => {
            reject_fractional_seconds(property, start_utc.naive_utc())?;
            Ok(format!("{}Z", start_utc.format("%Y%m%dT%H%M%S")))
        }
        TimeSpec::Month { .. } | TimeSpec::Year { .. } | TimeSpec::Unknown { .. } => {
            Err(IcalRecurrenceError::UnsupportedBaseKind(value.kind_name()))
        }
    }
}

fn reject_fractional_seconds(
    property: &'static str,
    value: NaiveDateTime,
) -> Result<(), IcalRecurrenceError> {
    if value.nanosecond() == 0 {
        Ok(())
    } else {
        Err(IcalRecurrenceError::FractionalSecondUnsupported {
            property,
            value: value.to_string(),
        })
    }
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

fn parse_byday_token(raw: &str) -> Result<(Option<i8>, RecurrenceWeekday), IcalRecurrenceError> {
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

fn parse_weekday_code(name: &str, raw: &str) -> Result<RecurrenceWeekday, IcalRecurrenceError> {
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

fn split_list<'a>(name: &str, raw: &'a str) -> Result<Vec<&'a str>, IcalRecurrenceError> {
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
    fn parses_date_rdate_and_preserves_date_only_duration() {
        let base = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 1, 10).expect("start"),
            end_exclusive: Some(NaiveDate::from_ymd_opt(2026, 1, 13).expect("end")),
        };
        let values =
            parse_rdate_property("RDATE;VALUE=DATE:20260310,20260415", &base).expect("RDATE");

        assert_eq!(
            values,
            vec![
                TimeSpec::DateOnly {
                    start: NaiveDate::from_ymd_opt(2026, 3, 10).expect("first"),
                    end_exclusive: Some(NaiveDate::from_ymd_opt(2026, 3, 13).expect("first end")),
                },
                TimeSpec::DateOnly {
                    start: NaiveDate::from_ymd_opt(2026, 4, 15).expect("second"),
                    end_exclusive: Some(NaiveDate::from_ymd_opt(2026, 4, 18).expect("second end")),
                },
            ]
        );
        assert_eq!(
            format_rdate_property(&values, &base).expect("format"),
            "RDATE;VALUE=DATE:20260310,20260415"
        );
    }

    #[test]
    fn parses_all_day_exdate_without_collapsing_kind() {
        let base = TimeSpec::AllDay {
            start: NaiveDate::from_ymd_opt(2026, 2, 1).expect("start"),
            end_exclusive: Some(NaiveDate::from_ymd_opt(2026, 2, 2).expect("end")),
        };
        let values =
            parse_exdate_property("EXDATE;VALUE=DATE:20260208", &base).expect("EXDATE");

        assert!(matches!(values.as_slice(), [TimeSpec::AllDay { .. }]));
        assert_eq!(
            format_exdate_property(&values, &base).expect("format"),
            "EXDATE;VALUE=DATE:20260208"
        );
    }

    #[test]
    fn parses_floating_rdate_and_preserves_duration_and_metadata() {
        let base = TimeSpec::Floating {
            start: NaiveDate::from_ymd_opt(2026, 3, 1)
                .expect("day")
                .and_hms_opt(9, 0, 0)
                .expect("time"),
            end: Some(
                NaiveDate::from_ymd_opt(2026, 3, 1)
                    .expect("day")
                    .and_hms_opt(10, 30, 0)
                    .expect("time"),
            ),
            source_timezone: Some("America/New_York".to_string()),
        };
        let values =
            parse_rdate_property("RDATE:20260308T090000,20260315T090000", &base).expect("RDATE");

        let TimeSpec::Floating {
            start,
            end,
            source_timezone,
        } = &values[0]
        else {
            panic!("floating");
        };
        assert_eq!(start.time().hour(), 9);
        assert_eq!(end.expect("end") - *start, Duration::minutes(90));
        assert_eq!(source_timezone.as_deref(), Some("America/New_York"));
        assert_eq!(
            format_rdate_property(&values, &base).expect("format"),
            "RDATE:20260308T090000,20260315T090000"
        );
    }

    #[test]
    fn parses_utc_instant_rdate_and_retains_series_source_timezone() {
        let base_start = DateTime::parse_from_rfc3339("2026-01-01T14:00:00Z")
            .expect("base")
            .with_timezone(&Utc);
        let base = TimeSpec::Instant {
            start_utc: base_start,
            end_utc: Some(base_start + Duration::hours(1)),
            source_timezone: Some("America/New_York".to_string()),
        };
        let values =
            parse_rdate_property("RDATE:20260702T130000Z", &base).expect("UTC RDATE");

        let TimeSpec::Instant {
            start_utc,
            end_utc,
            source_timezone,
        } = &values[0]
        else {
            panic!("instant");
        };
        assert_eq!(
            *start_utc,
            DateTime::parse_from_rfc3339("2026-07-02T13:00:00Z")
                .expect("expected")
                .with_timezone(&Utc)
        );
        assert_eq!(end_utc.expect("end") - *start_utc, Duration::hours(1));
        assert_eq!(source_timezone.as_deref(), Some("America/New_York"));
        assert_eq!(
            format_rdate_property(&values, &base).expect("format"),
            "RDATE:20260702T130000Z"
        );
    }

    #[test]
    fn parses_matching_tzid_instant_and_uses_first_ambiguous_occurrence() {
        let base_start = DateTime::parse_from_rfc3339("2026-01-01T14:00:00Z")
            .expect("base")
            .with_timezone(&Utc);
        let base = TimeSpec::Instant {
            start_utc: base_start,
            end_utc: None,
            source_timezone: Some("America/New_York".to_string()),
        };
        let values = parse_rdate_property(
            "RDATE;TZID=America/New_York:20261101T013000",
            &base,
        )
        .expect("TZID RDATE");

        let TimeSpec::Instant { start_utc, .. } = values[0] else {
            panic!("instant");
        };
        assert_eq!(
            start_utc,
            DateTime::parse_from_rfc3339("2026-11-01T05:30:00Z")
                .expect("first occurrence")
                .with_timezone(&Utc)
        );
    }

    #[test]
    fn resolves_nonexistent_tzid_local_time_with_pre_gap_offset() {
        let base_start = DateTime::parse_from_rfc3339("2026-01-01T14:00:00Z")
            .expect("base")
            .with_timezone(&Utc);
        let base = TimeSpec::Instant {
            start_utc: base_start,
            end_utc: None,
            source_timezone: Some("America/New_York".to_string()),
        };
        let values = parse_rdate_property(
            "RDATE;TZID=America/New_York:20260308T023000",
            &base,
        )
        .expect("gap RDATE");

        let TimeSpec::Instant { start_utc, .. } = values[0] else {
            panic!("instant");
        };
        assert_eq!(
            start_utc,
            DateTime::parse_from_rfc3339("2026-03-08T07:30:00Z")
                .expect("pre-gap offset")
                .with_timezone(&Utc)
        );
    }

    #[test]
    fn rejects_value_type_and_timezone_mismatches() {
        let date_base = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 1, 1).expect("date"),
            end_exclusive: None,
        };
        assert!(matches!(
            parse_rdate_property("RDATE:20260102T090000", &date_base),
            Err(IcalRecurrenceError::TemporalKindMismatch { .. })
        ));

        let floating_base = TimeSpec::Floating {
            start: NaiveDate::from_ymd_opt(2026, 1, 1)
                .expect("day")
                .and_hms_opt(9, 0, 0)
                .expect("time"),
            end: None,
            source_timezone: None,
        };
        assert!(matches!(
            parse_exdate_property(
                "EXDATE;TZID=America/New_York:20260102T090000",
                &floating_base
            ),
            Err(IcalRecurrenceError::TemporalKindMismatch { .. })
        ));

        let instant_base = TimeSpec::Instant {
            start_utc: DateTime::parse_from_rfc3339("2026-01-01T14:00:00Z")
                .expect("base")
                .with_timezone(&Utc),
            end_utc: None,
            source_timezone: Some("America/New_York".to_string()),
        };
        assert!(matches!(
            parse_rdate_property(
                "RDATE;TZID=Europe/London:20260102T090000",
                &instant_base
            ),
            Err(IcalRecurrenceError::TimezoneMismatch { .. })
        ));
    }

    #[test]
    fn accepts_quoted_matching_tzid_and_rejects_unknown_value_type() {
        let base = TimeSpec::Instant {
            start_utc: DateTime::parse_from_rfc3339("2026-01-01T14:00:00Z")
                .expect("base")
                .with_timezone(&Utc),
            end_utc: None,
            source_timezone: Some("America/New_York".to_string()),
        };
        let values = parse_exdate_property(
            "EXDATE;TZID=\"America/New_York\":20260702T090000",
            &base,
        )
        .expect("quoted TZID");
        assert_eq!(values.len(), 1);

        assert_eq!(
            parse_rdate_property("RDATE;VALUE=BOGUS:20260702T090000Z", &base),
            Err(IcalRecurrenceError::UnsupportedValueType(
                "BOGUS".to_string()
            ))
        );
    }

    #[test]
    fn rejects_rdate_period_and_unknown_parameters_without_lossy_fallback() {
        let base = TimeSpec::Instant {
            start_utc: DateTime::parse_from_rfc3339("2026-01-01T14:00:00Z")
                .expect("base")
                .with_timezone(&Utc),
            end_utc: None,
            source_timezone: None,
        };
        assert_eq!(
            parse_rdate_property(
                "RDATE;VALUE=PERIOD:20260102T090000Z/20260102T100000Z",
                &base
            ),
            Err(IcalRecurrenceError::UnsupportedPeriod)
        );
        assert!(matches!(
            parse_exdate_property("EXDATE;X-TEST=1:20260102T090000Z", &base),
            Err(IcalRecurrenceError::UnsupportedParameter(name)) if name == "X-TEST"
        ));
    }

    #[test]
    fn recurrence_date_parser_deduplicates_duplicate_values() {
        let base = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 1, 1).expect("date"),
            end_exclusive: None,
        };
        let values = parse_rdate_property(
            "RDATE;VALUE=DATE:20260102,20260102,20260103",
            &base,
        )
        .expect("RDATE");
        assert_eq!(values.len(), 2);
    }

    #[test]
    fn recurrence_date_formatter_rejects_shape_or_fractional_second_loss() {
        let base = TimeSpec::Floating {
            start: NaiveDate::from_ymd_opt(2026, 1, 1)
                .expect("day")
                .and_hms_opt(9, 0, 0)
                .expect("time"),
            end: Some(
                NaiveDate::from_ymd_opt(2026, 1, 1)
                    .expect("day")
                    .and_hms_opt(10, 0, 0)
                    .expect("time"),
            ),
            source_timezone: None,
        };
        let wrong_duration = TimeSpec::Floating {
            start: NaiveDate::from_ymd_opt(2026, 1, 2)
                .expect("day")
                .and_hms_opt(9, 0, 0)
                .expect("time"),
            end: Some(
                NaiveDate::from_ymd_opt(2026, 1, 2)
                    .expect("day")
                    .and_hms_opt(11, 0, 0)
                    .expect("time"),
            ),
            source_timezone: None,
        };
        assert!(matches!(
            format_rdate_property(&[wrong_duration], &base),
            Err(IcalRecurrenceError::ExceptionShapeMismatch { .. })
        ));

        let fractional = TimeSpec::Floating {
            start: NaiveDate::from_ymd_opt(2026, 1, 2)
                .expect("day")
                .and_hms_nano_opt(9, 0, 0, 1)
                .expect("time"),
            end: Some(
                NaiveDate::from_ymd_opt(2026, 1, 2)
                    .expect("day")
                    .and_hms_nano_opt(10, 0, 0, 1)
                    .expect("time"),
            ),
            source_timezone: None,
        };
        assert!(matches!(
            format_rdate_property(&[fractional], &base),
            Err(IcalRecurrenceError::FractionalSecondUnsupported { .. })
        ));
    }

    #[test]
    fn recurrence_date_property_requires_expected_name_and_nonempty_values() {
        let base = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 1, 1).expect("date"),
            end_exclusive: None,
        };
        assert!(matches!(
            parse_rdate_property("EXDATE;VALUE=DATE:20260102", &base),
            Err(IcalRecurrenceError::WrongProperty { .. })
        ));
        assert_eq!(
            parse_rdate_property("RDATE;VALUE=DATE:", &base),
            Err(IcalRecurrenceError::EmptyPropertyValues("RDATE"))
        );
    }

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
            Err(IcalRecurrenceError::Domain(
                RecurrenceError::InvalidByMonth(13)
            ))
        ));
        assert!(matches!(
            parse_rrule("FREQ=DAILY;BYWEEKNO=1"),
            Err(IcalRecurrenceError::Domain(
                RecurrenceError::ByWeekNoRequiresYearly
            ))
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
            "FREQ=YEARLY;INTERVAL=3;BYMONTH=1,7;BYDAY=MO,-1FR;BYHOUR=9;BYMINUTE=30;BYSETPOS=-1",
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
