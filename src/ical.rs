use std::{collections::HashSet, fmt};

use chrono::{
    DateTime, Duration, LocalResult, NaiveDate, NaiveDateTime, Offset, TimeZone, Timelike, Utc,
};
use chrono_tz::Tz;
use serde_json::json;

use crate::domain::{
    EventStatus, RecurrenceError, RecurrenceFrequency, RecurrenceOrdinalWeekday,
    RecurrenceOverride, RecurrenceRule, RecurrenceWeekday, TemporalEvent, TimeSpec,
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
    InvalidProperty(String),
    WrongProperty {
        expected: &'static str,
        actual: String,
    },
    DuplicateParameter(String),
    UnsupportedParameter(String),
    UnsupportedValueType(String),
    UnsupportedPeriod,
    UnsupportedRecurrenceRange(String),
    RecurrenceIdRequiresSingleValue,
    EventTimeRequiresSingleValue(&'static str),
    UnsupportedVeventTimeKind(&'static str),
    InvalidContentLine(String),
    InvalidTextEscape(String),
    InvalidComponent(String),
    MissingProperty(&'static str),
    DuplicateProperty(String),
    UnsupportedProperty(String),
    MasterHasRecurrenceId,
    DetachedMissingRecurrenceId,
    UidMismatch {
        expected: String,
        actual: String,
    },
    UnsupportedRecurrenceSet(String),
    UnsupportedDetachedOverride(String),
    UnsupportedComponent(String),
    MissingMaster(String),
    DuplicateMaster(String),
    UnsupportedCanonicalStatus(String),
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
            Self::UnsupportedRecurrenceRange(value) => write!(
                formatter,
                "RECURRENCE-ID RANGE={value} is not supported until range overrides are canonical"
            ),
            Self::RecurrenceIdRequiresSingleValue => {
                formatter.write_str("RECURRENCE-ID requires exactly one date or date-time value")
            }
            Self::EventTimeRequiresSingleValue(property) => {
                write!(formatter, "{property} requires exactly one date or date-time value")
            }
            Self::UnsupportedVeventTimeKind(kind) => {
                write!(formatter, "VEVENT transport cannot losslessly represent {kind}")
            }
            Self::InvalidContentLine(value) => {
                write!(formatter, "invalid iCalendar content line: {value}")
            }
            Self::InvalidTextEscape(value) => {
                write!(formatter, "invalid iCalendar TEXT escape in {value}")
            }
            Self::InvalidComponent(value) => {
                write!(formatter, "invalid iCalendar component: {value}")
            }
            Self::MissingProperty(name) => write!(formatter, "VEVENT requires {name}"),
            Self::DuplicateProperty(name) => write!(formatter, "duplicate VEVENT property {name}"),
            Self::UnsupportedProperty(name) => write!(formatter, "unsupported VEVENT property {name}"),
            Self::MasterHasRecurrenceId => {
                formatter.write_str("master VEVENT must not contain RECURRENCE-ID")
            }
            Self::DetachedMissingRecurrenceId => {
                formatter.write_str("detached VEVENT requires RECURRENCE-ID")
            }
            Self::UidMismatch { expected, actual } => write!(
                formatter,
                "detached VEVENT UID {actual} does not match master UID {expected}"
            ),
            Self::UnsupportedRecurrenceSet(value) => {
                write!(formatter, "unsupported VEVENT recurrence set: {value}")
            }
            Self::UnsupportedDetachedOverride(value) => {
                write!(formatter, "unsupported detached VEVENT override: {value}")
            }
            Self::UnsupportedComponent(value) => {
                write!(formatter, "unsupported iCalendar component {value}")
            }
            Self::MissingMaster(uid) => {
                write!(formatter, "VEVENT UID {uid} has detached instances but no master")
            }
            Self::DuplicateMaster(uid) => {
                write!(formatter, "VEVENT UID {uid} has more than one master component")
            }
            Self::UnsupportedCanonicalStatus(status) => {
                write!(formatter, "canonical event status {status} has no lossless VEVENT STATUS mapping")
            }
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IcalContentLine {
    pub name: String,
    /// Parameter values are retained in their serialized form, including
    /// surrounding quotes when the source used quoted-string syntax.
    pub parameters: Vec<(String, String)>,
    pub value: String,
}

/// Parse one unfolded RFC 5545 content line.
///
/// The head/value separator and parameter separators are recognized only
/// outside quoted parameter values, so values such as ALTREP="cid:part:1"
/// do not corrupt the parse.
pub fn parse_ical_content_line(raw: &str) -> Result<IcalContentLine, IcalRecurrenceError> {
    if raw.is_empty() || raw.contains(['\r', '\n']) {
        return Err(IcalRecurrenceError::InvalidContentLine(raw.to_string()));
    }
    let separator = find_unquoted(raw, ':')
        .ok_or_else(|| IcalRecurrenceError::InvalidContentLine(raw.to_string()))?;
    let head = &raw[..separator];
    let value = &raw[separator + 1..];
    let head_parts = split_unquoted(head, ';')?;
    let Some((raw_name, raw_parameters)) = head_parts.split_first() else {
        return Err(IcalRecurrenceError::InvalidContentLine(raw.to_string()));
    };
    let name = raw_name.trim().to_ascii_uppercase();
    validate_ical_token(&name, raw)?;

    let mut parameters = Vec::with_capacity(raw_parameters.len());
    for raw_parameter in raw_parameters {
        let (raw_parameter_name, raw_parameter_value) = raw_parameter
            .split_once('=')
            .ok_or_else(|| IcalRecurrenceError::InvalidContentLine(raw.to_string()))?;
        let parameter_name = raw_parameter_name.trim().to_ascii_uppercase();
        validate_ical_token(&parameter_name, raw)?;
        let parameter_value = raw_parameter_value.trim();
        if parameter_value.is_empty()
            || parameter_value.contains(['\r', '\n'])
            || !valid_parameter_quoting(parameter_value)
        {
            return Err(IcalRecurrenceError::InvalidContentLine(raw.to_string()));
        }
        parameters.push((parameter_name, parameter_value.to_string()));
    }

    Ok(IcalContentLine {
        name,
        parameters,
        value: value.to_string(),
    })
}

pub fn format_ical_content_line(line: &IcalContentLine) -> Result<String, IcalRecurrenceError> {
    validate_ical_token(&line.name, &line.name)?;
    if line.value.contains(['\r', '\n']) {
        return Err(IcalRecurrenceError::InvalidContentLine(line.value.clone()));
    }

    let mut encoded = line.name.to_ascii_uppercase();
    for (name, value) in &line.parameters {
        validate_ical_token(name, name)?;
        if value.is_empty() || value.contains(['\r', '\n']) || !valid_parameter_quoting(value) {
            return Err(IcalRecurrenceError::InvalidContentLine(format!(
                "{name}={value}"
            )));
        }
        encoded.push(';');
        encoded.push_str(&name.to_ascii_uppercase());
        encoded.push('=');
        encoded.push_str(value);
    }
    encoded.push(':');
    encoded.push_str(&line.value);
    Ok(encoded)
}

/// Unfold RFC 5545 physical lines into logical content lines.
pub fn unfold_ical_content_lines(raw: &str) -> Result<Vec<String>, IcalRecurrenceError> {
    let normalized = raw.replace("\r\n", "\n");
    if normalized.contains('\r') {
        return Err(IcalRecurrenceError::InvalidContentLine(
            "bare carriage return".to_string(),
        ));
    }

    let mut lines: Vec<String> = Vec::new();
    for physical in normalized.split('\n') {
        if physical.is_empty() {
            continue;
        }
        if let Some(rest) = physical
            .strip_prefix(' ')
            .or_else(|| physical.strip_prefix('\t'))
        {
            let previous = lines.last_mut().ok_or_else(|| {
                IcalRecurrenceError::InvalidContentLine(
                    "folded continuation without a preceding line".to_string(),
                )
            })?;
            previous.push_str(rest);
        } else {
            lines.push(physical.to_string());
        }
    }
    Ok(lines)
}

/// Fold one logical content line at RFC 5545's 75-octet boundary without
/// splitting UTF-8 code points. Continuation payloads use 74 octets because
/// the leading SPACE counts toward the physical-line limit.
pub fn fold_ical_content_line(raw: &str) -> Result<String, IcalRecurrenceError> {
    if raw.contains(['\r', '\n']) {
        return Err(IcalRecurrenceError::InvalidContentLine(raw.to_string()));
    }
    if raw.len() <= 75 {
        return Ok(raw.to_string());
    }

    let mut remaining = raw;
    let mut limit = 75_usize;
    let mut encoded = String::new();
    while !remaining.is_empty() {
        let cut = utf8_prefix_len(remaining, limit);
        if cut == 0 {
            return Err(IcalRecurrenceError::InvalidContentLine(raw.to_string()));
        }
        encoded.push_str(&remaining[..cut]);
        remaining = &remaining[cut..];
        if !remaining.is_empty() {
            encoded.push_str("\r\n ");
            limit = 74;
        }
    }
    Ok(encoded)
}

/// Encode an RFC 5545 TEXT value.
pub fn escape_ical_text(raw: &str) -> String {
    let normalized = raw.replace("\r\n", "\n").replace('\r', "\n");
    let mut encoded = String::with_capacity(normalized.len());
    for character in normalized.chars() {
        match character {
            '\\' => encoded.push_str("\\\\"),
            '\n' => encoded.push_str("\\n"),
            ';' => encoded.push_str("\\;"),
            ',' => encoded.push_str("\\,"),
            _ => encoded.push(character),
        }
    }
    encoded
}

/// Decode an RFC 5545 TEXT value, rejecting undefined backslash escapes.
pub fn unescape_ical_text(raw: &str) -> Result<String, IcalRecurrenceError> {
    let mut decoded = String::with_capacity(raw.len());
    let mut characters = raw.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            decoded.push(character);
            continue;
        }
        let escaped = characters
            .next()
            .ok_or_else(|| IcalRecurrenceError::InvalidTextEscape(raw.to_string()))?;
        match escaped {
            '\\' => decoded.push('\\'),
            'n' | 'N' => decoded.push('\n'),
            ';' => decoded.push(';'),
            ',' => decoded.push(','),
            _ => return Err(IcalRecurrenceError::InvalidTextEscape(raw.to_string())),
        }
    }
    Ok(decoded)
}

/// Parse exactly one VEVENT envelope into unfolded generic content lines.
///
/// Semantic interpretation is deliberately layered above this function; this
/// boundary preserves unknown properties instead of silently discarding them.
pub fn parse_vevent_content_lines(raw: &str) -> Result<Vec<IcalContentLine>, IcalRecurrenceError> {
    let lines = unfold_ical_content_lines(raw)?;
    if lines.len() < 2
        || !lines[0].eq_ignore_ascii_case("BEGIN:VEVENT")
        || !lines[lines.len() - 1].eq_ignore_ascii_case("END:VEVENT")
    {
        return Err(IcalRecurrenceError::InvalidComponent(
            "expected exactly one BEGIN:VEVENT ... END:VEVENT envelope".to_string(),
        ));
    }

    let mut parsed = Vec::with_capacity(lines.len().saturating_sub(2));
    for line in &lines[1..lines.len() - 1] {
        let content = parse_ical_content_line(line)?;
        if matches!(content.name.as_str(), "BEGIN" | "END") {
            return Err(IcalRecurrenceError::InvalidComponent(
                "nested components are not allowed in the single-VEVENT envelope".to_string(),
            ));
        }
        parsed.push(content);
    }
    Ok(parsed)
}

/// Serialize one VEVENT envelope using CRLF and UTF-8-safe RFC line folding.
pub fn format_vevent_content_lines(
    lines: &[IcalContentLine],
) -> Result<String, IcalRecurrenceError> {
    let mut physical = Vec::with_capacity(lines.len() + 2);
    physical.push("BEGIN:VEVENT".to_string());
    for line in lines {
        if matches!(line.name.to_ascii_uppercase().as_str(), "BEGIN" | "END") {
            return Err(IcalRecurrenceError::InvalidComponent(
                "component marker supplied as a VEVENT property".to_string(),
            ));
        }
        physical.push(fold_ical_content_line(&format_ical_content_line(line)?)?);
    }
    physical.push("END:VEVENT".to_string());
    Ok(format!("{}\r\n", physical.join("\r\n")))
}

/// Export one canonical TemporalEvent into its VEVENT master and detached
/// recurrence components.
///
/// Required RFC identity fields are deterministic: imported iCalendar UID is
/// reused when available; otherwise the canonical UUID becomes a URN UID.
/// DTSTAMP uses the event's canonical updated_at truncated to RFC seconds.
pub fn export_temporal_event(
    event: &TemporalEvent,
) -> Result<Vec<IcalVevent>, IcalRecurrenceError> {
    event.validate_recurrence()?;

    let template = stored_ical_master(event)?;
    let uid = stored_ical_uid(event)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("urn:uuid:{}", event.id));
    let dtstamp = DateTime::<Utc>::from_timestamp(event.updated_at.timestamp(), 0)
        .ok_or(IcalRecurrenceError::ArithmeticOverflow)?;
    let status = export_vevent_status(event.status)?;

    let imported_untitled = template
        .as_ref()
        .is_some_and(|source| source.summary.is_none() && event.normalized_title == uid);
    let summary = if imported_untitled {
        None
    } else {
        Some(IcalProperty {
            value: event.normalized_title.clone(),
            parameters: template
                .as_ref()
                .and_then(|source| source.summary.as_ref())
                .map_or_else(Vec::new, |property| property.parameters.clone()),
        })
    };

    let description = event.description.as_ref().map(|value| IcalProperty {
        value: value.clone(),
        parameters: template
            .as_ref()
            .and_then(|source| source.description.as_ref())
            .map_or_else(Vec::new, |property| property.parameters.clone()),
    });

    let mut master = IcalVevent {
        uid: IcalProperty {
            value: uid.clone(),
            parameters: template
                .as_ref()
                .map_or_else(Vec::new, |source| source.uid.parameters.clone()),
        },
        dtstamp: IcalProperty {
            value: dtstamp,
            parameters: Vec::new(),
        },
        time: event.time.clone(),
        summary,
        description,
        status: status.map(|value| IcalProperty {
            value,
            parameters: template
                .as_ref()
                .and_then(|source| source.status.as_ref())
                .map_or_else(Vec::new, |property| property.parameters.clone()),
        }),
        sequence: template
            .as_ref()
            .and_then(|source| source.sequence.as_ref())
            .cloned(),
        rrule: None,
        rdates: Vec::new(),
        exdates: Vec::new(),
        recurrence_id: None,
        extra_properties: template
            .as_ref()
            .map_or_else(Vec::new, |source| source.extra_properties.clone()),
    };

    let Some(rule) = event.recurrence.as_ref() else {
        // Force the time boundary now so unsupported DateOnly/imprecise kinds
        // fail during canonical export rather than only during final formatting.
        format_vevent_time_properties(&master.time)?;
        return Ok(vec![master]);
    };

    let mut core_rule = rule.clone();
    core_rule.rdates.clear();
    core_rule.exdates.clear();
    core_rule.overrides.clear();
    master.rrule = Some(IcalProperty {
        value: core_rule,
        parameters: template
            .as_ref()
            .and_then(|source| source.rrule.as_ref())
            .map_or_else(Vec::new, |property| property.parameters.clone()),
    });
    master.rdates = rule.rdates.clone();
    master.exdates = rule.exdates.clone();

    // Validate every serializable boundary before building detached components.
    format_vevent(&master)?;

    let mut components = Vec::with_capacity(rule.overrides.len() + 1);
    components.push(master.clone());

    for occurrence_override in &rule.overrides {
        let recurrence_id = parse_ical_content_line(&format_recurrence_id_property(
            &occurrence_override.original,
            &event.time,
        )?)?;
        let time = occurrence_override
            .replacement
            .clone()
            .unwrap_or_else(|| occurrence_override.original.clone());
        let detached = IcalVevent {
            uid: IcalProperty {
                value: uid.clone(),
                parameters: master.uid.parameters.clone(),
            },
            dtstamp: IcalProperty {
                value: dtstamp,
                parameters: Vec::new(),
            },
            time,
            summary: None,
            description: None,
            status: occurrence_override.cancelled.then_some(IcalProperty {
                value: IcalVeventStatus::Cancelled,
                parameters: Vec::new(),
            }),
            sequence: master.sequence.clone(),
            rrule: None,
            rdates: Vec::new(),
            exdates: Vec::new(),
            recurrence_id: Some(recurrence_id),
            extra_properties: Vec::new(),
        };
        format_vevent(&detached)?;
        components.push(detached);
    }

    Ok(components)
}

/// Build a strict RFC 5545 calendar from canonical events.
pub fn export_temporal_events_vcalendar(
    events: &[TemporalEvent],
    prodid: &str,
) -> Result<IcalVcalendar, IcalRecurrenceError> {
    if prodid.trim().is_empty() {
        return Err(IcalRecurrenceError::InvalidPropertyValue {
            property: "PRODID",
            value: prodid.to_string(),
        });
    }

    let mut vevents = Vec::new();
    for event in events {
        vevents.extend(export_temporal_event(event)?);
    }

    Ok(IcalVcalendar {
        properties: vec![
            IcalContentLine {
                name: "PRODID".to_string(),
                parameters: Vec::new(),
                value: prodid.to_string(),
            },
            IcalContentLine {
                name: "VERSION".to_string(),
                parameters: Vec::new(),
                value: "2.0".to_string(),
            },
            IcalContentLine {
                name: "CALSCALE".to_string(),
                parameters: Vec::new(),
                value: "GREGORIAN".to_string(),
            },
        ],
        events: vevents,
    })
}

pub fn format_temporal_events_vcalendar(
    events: &[TemporalEvent],
    prodid: &str,
) -> Result<String, IcalRecurrenceError> {
    format_vcalendar(&export_temporal_events_vcalendar(events, prodid)?)
}

fn stored_ical_uid(event: &TemporalEvent) -> Option<&str> {
    event
        .properties
        .get("ical")
        .and_then(|value| value.get("uid"))
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
}

fn stored_ical_master(event: &TemporalEvent) -> Result<Option<IcalVevent>, IcalRecurrenceError> {
    let Some(raw) = event
        .properties
        .get("ical")
        .and_then(|value| value.get("master_component"))
        .and_then(|value| value.as_str())
    else {
        return Ok(None);
    };
    Ok(Some(parse_vevent(raw)?))
}

fn export_vevent_status(
    status: EventStatus,
) -> Result<Option<IcalVeventStatus>, IcalRecurrenceError> {
    match status {
        EventStatus::Scheduled => Ok(None),
        EventStatus::Tentative => Ok(Some(IcalVeventStatus::Tentative)),
        EventStatus::Confirmed => Ok(Some(IcalVeventStatus::Confirmed)),
        EventStatus::Cancelled => Ok(Some(IcalVeventStatus::Cancelled)),
        other => Err(IcalRecurrenceError::UnsupportedCanonicalStatus(
            other.as_str().to_string(),
        )),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct IcalVcalendar {
    pub properties: Vec<IcalContentLine>,
    pub events: Vec<IcalVevent>,
}

/// Parse one RFC 5545 VCALENDAR containing top-level properties and VEVENTs.
///
/// Nested component families are intentionally rejected until they have an
/// explicit preservation/semantic policy; they are never silently skipped.
pub fn parse_vcalendar(raw: &str) -> Result<IcalVcalendar, IcalRecurrenceError> {
    let lines = unfold_ical_content_lines(raw)?;
    if lines.len() < 2
        || !lines[0].eq_ignore_ascii_case("BEGIN:VCALENDAR")
        || !lines[lines.len() - 1].eq_ignore_ascii_case("END:VCALENDAR")
    {
        return Err(IcalRecurrenceError::InvalidComponent(
            "expected BEGIN:VCALENDAR ... END:VCALENDAR envelope".to_string(),
        ));
    }

    let mut properties = Vec::new();
    let mut events = Vec::new();
    let mut index = 1_usize;
    while index < lines.len() - 1 {
        let line = &lines[index];
        if line.eq_ignore_ascii_case("BEGIN:VEVENT") {
            let start = index;
            index += 1;
            while index < lines.len() - 1 && !lines[index].eq_ignore_ascii_case("END:VEVENT") {
                if let Some(component) = lines[index].strip_prefix("BEGIN:") {
                    return Err(IcalRecurrenceError::UnsupportedComponent(
                        component.to_ascii_uppercase(),
                    ));
                }
                index += 1;
            }
            if index >= lines.len() - 1 {
                return Err(IcalRecurrenceError::InvalidComponent(
                    "unterminated VEVENT".to_string(),
                ));
            }
            let raw_event = format!("{}\r\n", lines[start..=index].join("\r\n"));
            events.push(parse_vevent(&raw_event)?);
            index += 1;
            continue;
        }

        if let Some(component) = line.strip_prefix("BEGIN:") {
            return Err(IcalRecurrenceError::UnsupportedComponent(
                component.to_ascii_uppercase(),
            ));
        }
        if line.starts_with("END:") {
            return Err(IcalRecurrenceError::InvalidComponent(format!(
                "unexpected top-level component terminator {line}"
            )));
        }

        properties.push(parse_ical_content_line(line)?);
        index += 1;
    }

    validate_vcalendar_properties(&properties)?;
    Ok(IcalVcalendar { properties, events })
}

pub fn format_vcalendar(calendar: &IcalVcalendar) -> Result<String, IcalRecurrenceError> {
    validate_vcalendar_properties(&calendar.properties)?;

    let mut encoded = String::from("BEGIN:VCALENDAR\r\n");
    for property in &calendar.properties {
        if matches!(property.name.as_str(), "BEGIN" | "END") {
            return Err(IcalRecurrenceError::InvalidComponent(
                "component marker supplied as VCALENDAR property".to_string(),
            ));
        }
        encoded.push_str(&fold_ical_content_line(&format_ical_content_line(
            property,
        )?)?);
        encoded.push_str("\r\n");
    }
    for event in &calendar.events {
        encoded.push_str(&format_vevent(event)?);
    }
    encoded.push_str("END:VCALENDAR\r\n");
    Ok(encoded)
}

impl IcalVcalendar {
    /// Group VEVENTs by UID, bind detached recurrence instances to their one
    /// master component, and project the supported payload into canonical
    /// TemporalEvents.
    pub fn canonical_events(&self) -> Result<Vec<TemporalEvent>, IcalRecurrenceError> {
        let mut canonical = Vec::new();
        let mut consumed = vec![false; self.events.len()];

        for index in 0..self.events.len() {
            if consumed[index] {
                continue;
            }
            let uid = self.events[index].uid.value.clone();
            let mut master = None;
            let mut detached = Vec::new();

            for (candidate_index, candidate) in self.events.iter().enumerate().skip(index) {
                if consumed[candidate_index] || candidate.uid.value != uid {
                    continue;
                }
                consumed[candidate_index] = true;
                if candidate.recurrence_id.is_some() {
                    detached.push(candidate.clone());
                } else if master.replace(candidate.clone()).is_some() {
                    return Err(IcalRecurrenceError::DuplicateMaster(uid));
                }
            }

            let master = master.ok_or_else(|| IcalRecurrenceError::MissingMaster(uid.clone()))?;
            let event = if detached.is_empty() {
                master.canonical_event()?
            } else {
                bind_vevent_series(master, detached)?.canonical_event()?
            };
            canonical.push(event);
        }

        Ok(canonical)
    }
}

fn validate_vcalendar_properties(
    properties: &[IcalContentLine],
) -> Result<(), IcalRecurrenceError> {
    let versions = properties
        .iter()
        .filter(|property| property.name == "VERSION")
        .collect::<Vec<_>>();
    if versions.is_empty() {
        return Err(IcalRecurrenceError::MissingProperty("VERSION"));
    }
    if versions.len() > 1 {
        return Err(IcalRecurrenceError::DuplicateProperty(
            "VERSION".to_string(),
        ));
    }
    if !versions[0].parameters.is_empty() || versions[0].value != "2.0" {
        return Err(IcalRecurrenceError::InvalidPropertyValue {
            property: "VERSION",
            value: versions[0].value.clone(),
        });
    }

    let prodids = properties
        .iter()
        .filter(|property| property.name == "PRODID")
        .collect::<Vec<_>>();
    if prodids.is_empty() {
        return Err(IcalRecurrenceError::MissingProperty("PRODID"));
    }
    if prodids.len() > 1 {
        return Err(IcalRecurrenceError::DuplicateProperty("PRODID".to_string()));
    }
    if !prodids[0].parameters.is_empty() || prodids[0].value.is_empty() {
        return Err(IcalRecurrenceError::InvalidPropertyValue {
            property: "PRODID",
            value: prodids[0].value.clone(),
        });
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
pub struct IcalProperty<T> {
    pub value: T,
    pub parameters: Vec<(String, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IcalVeventStatus {
    Tentative,
    Confirmed,
    Cancelled,
}

impl IcalVeventStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Tentative => "TENTATIVE",
            Self::Confirmed => "CONFIRMED",
            Self::Cancelled => "CANCELLED",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_uppercase().as_str() {
            "TENTATIVE" => Some(Self::Tentative),
            "CONFIRMED" => Some(Self::Confirmed),
            "CANCELLED" => Some(Self::Cancelled),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct IcalVevent {
    pub uid: IcalProperty<String>,
    pub dtstamp: IcalProperty<DateTime<Utc>>,
    pub time: TimeSpec,
    pub summary: Option<IcalProperty<String>>,
    pub description: Option<IcalProperty<String>>,
    pub status: Option<IcalProperty<IcalVeventStatus>>,
    pub sequence: Option<IcalProperty<u32>>,
    pub rrule: Option<IcalProperty<RecurrenceRule>>,
    pub rdates: Vec<TimeSpec>,
    pub exdates: Vec<TimeSpec>,
    /// Kept as a parsed content line until a master VEVENT supplies the
    /// canonical DTSTART shape needed to decode original-slot identity.
    pub recurrence_id: Option<IcalContentLine>,
    pub extra_properties: Vec<IcalContentLine>,
}

/// Bind a single VEVENT component into typed RFC properties while preserving
/// unknown non-temporal properties for later source-aware handling.
pub fn parse_vevent(raw: &str) -> Result<IcalVevent, IcalRecurrenceError> {
    let lines = parse_vevent_content_lines(raw)?;

    let mut uid = None;
    let mut dtstamp = None;
    let mut dtstart = None;
    let mut dtend = None;
    let mut summary = None;
    let mut description = None;
    let mut status = None;
    let mut sequence = None;
    let mut rrule = None;
    let mut rdate_lines = Vec::new();
    let mut exdate_lines = Vec::new();
    let mut recurrence_id = None;
    let mut extra_properties = Vec::new();

    for line in lines {
        match line.name.as_str() {
            "UID" => set_unique_content_line(&mut uid, line)?,
            "DTSTAMP" => set_unique_content_line(&mut dtstamp, line)?,
            "DTSTART" => set_unique_content_line(&mut dtstart, line)?,
            "DTEND" => set_unique_content_line(&mut dtend, line)?,
            "SUMMARY" => set_unique_content_line(&mut summary, line)?,
            "DESCRIPTION" => set_unique_content_line(&mut description, line)?,
            "STATUS" => set_unique_content_line(&mut status, line)?,
            "SEQUENCE" => set_unique_content_line(&mut sequence, line)?,
            "RRULE" => set_unique_content_line(&mut rrule, line)?,
            "RDATE" => rdate_lines.push(line),
            "EXDATE" => exdate_lines.push(line),
            "RECURRENCE-ID" => set_unique_content_line(&mut recurrence_id, line)?,
            "DURATION" => {
                return Err(IcalRecurrenceError::UnsupportedProperty(
                    "DURATION".to_string(),
                ));
            }
            _ => extra_properties.push(line),
        }
    }

    let uid_line = uid.ok_or(IcalRecurrenceError::MissingProperty("UID"))?;
    let uid = parse_text_property(uid_line)?;
    if uid.value.is_empty() {
        return Err(IcalRecurrenceError::InvalidPropertyValue {
            property: "UID",
            value: String::new(),
        });
    }

    let dtstamp_line = dtstamp.ok_or(IcalRecurrenceError::MissingProperty("DTSTAMP"))?;
    let dtstamp = parse_dtstamp_property(dtstamp_line)?;

    let dtstart_line = dtstart.ok_or(IcalRecurrenceError::MissingProperty("DTSTART"))?;
    let dtstart_raw = format_ical_content_line(&dtstart_line)?;
    let dtend_raw = dtend.as_ref().map(format_ical_content_line).transpose()?;
    let time = parse_vevent_time_properties(&dtstart_raw, dtend_raw.as_deref())?;

    let summary = summary.map(parse_text_property).transpose()?;
    let description = description.map(parse_text_property).transpose()?;
    let status = status.map(parse_status_property).transpose()?;
    let sequence = sequence.map(parse_sequence_property).transpose()?;
    let rrule = rrule.map(parse_rrule_property).transpose()?;

    let mut rdates = Vec::new();
    for line in rdate_lines {
        rdates.extend(parse_rdate_property(
            &format_ical_content_line(&line)?,
            &time,
        )?);
    }
    let mut exdates = Vec::new();
    for line in exdate_lines {
        exdates.extend(parse_exdate_property(
            &format_ical_content_line(&line)?,
            &time,
        )?);
    }

    Ok(IcalVevent {
        uid,
        dtstamp,
        time,
        summary,
        description,
        status,
        sequence,
        rrule,
        rdates,
        exdates,
        recurrence_id,
        extra_properties,
    })
}

/// Serialize a typed VEVENT back through the generic envelope while preserving
/// parameterized text/identity properties and unknown properties.
pub fn format_vevent(event: &IcalVevent) -> Result<String, IcalRecurrenceError> {
    let mut lines = Vec::new();
    lines.push(format_text_property_line("UID", &event.uid));

    reject_fractional_seconds("DTSTAMP", event.dtstamp.value.naive_utc())?;
    lines.push(IcalContentLine {
        name: "DTSTAMP".to_string(),
        parameters: event.dtstamp.parameters.clone(),
        value: format!("{}Z", event.dtstamp.value.format("%Y%m%dT%H%M%S")),
    });

    let (dtstart, dtend) = format_vevent_time_properties(&event.time)?;
    lines.push(parse_ical_content_line(&dtstart)?);
    if let Some(dtend) = dtend {
        lines.push(parse_ical_content_line(&dtend)?);
    }

    if let Some(recurrence_id) = &event.recurrence_id {
        if recurrence_id.name != "RECURRENCE-ID" {
            return Err(IcalRecurrenceError::InvalidComponent(
                "typed recurrence_id is not a RECURRENCE-ID property".to_string(),
            ));
        }
        lines.push(recurrence_id.clone());
    }

    if let Some(rrule) = &event.rrule {
        lines.push(IcalContentLine {
            name: "RRULE".to_string(),
            parameters: rrule.parameters.clone(),
            value: format_rrule(&rrule.value)?,
        });
    }
    if !event.rdates.is_empty() {
        lines.push(parse_ical_content_line(&format_rdate_property(
            &event.rdates,
            &event.time,
        )?)?);
    }
    if !event.exdates.is_empty() {
        lines.push(parse_ical_content_line(&format_exdate_property(
            &event.exdates,
            &event.time,
        )?)?);
    }

    if let Some(summary) = &event.summary {
        lines.push(format_text_property_line("SUMMARY", summary));
    }
    if let Some(description) = &event.description {
        lines.push(format_text_property_line("DESCRIPTION", description));
    }
    if let Some(status) = &event.status {
        lines.push(IcalContentLine {
            name: "STATUS".to_string(),
            parameters: status.parameters.clone(),
            value: status.value.as_str().to_string(),
        });
    }
    if let Some(sequence) = &event.sequence {
        lines.push(IcalContentLine {
            name: "SEQUENCE".to_string(),
            parameters: sequence.parameters.clone(),
            value: sequence.value.to_string(),
        });
    }

    for extra in &event.extra_properties {
        if is_reserved_typed_vevent_property(&extra.name) {
            return Err(IcalRecurrenceError::DuplicateProperty(extra.name.clone()));
        }
        lines.push(extra.clone());
    }

    format_vevent_content_lines(&lines)
}

impl IcalVevent {
    /// Project one master or non-recurring VEVENT into the canonical event
    /// model. Detached instances require master context and are rejected here.
    pub fn canonical_event(&self) -> Result<TemporalEvent, IcalRecurrenceError> {
        if self.recurrence_id.is_some() {
            return Err(IcalRecurrenceError::DetachedMissingRecurrenceId);
        }

        let recurrence = match self.rrule.as_ref() {
            Some(rrule) => {
                let mut rule = rrule.value.clone();
                rule.rdates = self.rdates.clone();
                rule.exdates = self.exdates.clone();
                rule.overrides.clear();
                Some(rule)
            }
            None if self.rdates.is_empty() && self.exdates.is_empty() => None,
            None => {
                return Err(IcalRecurrenceError::UnsupportedRecurrenceSet(
                    "RDATE/EXDATE without RRULE is not representable by the canonical recurrence model"
                        .to_string(),
                ));
            }
        };

        let title = self
            .summary
            .as_ref()
            .map(|property| property.value.clone())
            .unwrap_or_else(|| self.uid.value.clone());
        let mut event = TemporalEvent::new(title, self.time.clone());
        event.source_record_key = Some(self.uid.value.clone());
        event.raw_title = self.summary.as_ref().map(|property| property.value.clone());
        event.description = self
            .description
            .as_ref()
            .map(|property| property.value.clone());
        event.status = canonical_status(self.status.as_ref().map(|property| property.value));
        event.recurrence = recurrence;
        event.properties = json!({
            "ical": {
                "transport": "rfc5545_vevent",
                "uid": self.uid.value,
                "dtstamp": self.dtstamp.value.to_rfc3339(),
                "sequence": self.sequence.as_ref().map(|property| property.value),
                "master_component": format_vevent(self)?,
                "detached_components": [],
            }
        });
        event.validate_recurrence()?;
        Ok(event)
    }
}

fn canonical_status(status: Option<IcalVeventStatus>) -> EventStatus {
    match status {
        Some(IcalVeventStatus::Tentative) => EventStatus::Tentative,
        Some(IcalVeventStatus::Confirmed) => EventStatus::Confirmed,
        Some(IcalVeventStatus::Cancelled) => EventStatus::Cancelled,
        None => EventStatus::Scheduled,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct IcalBoundDetachedVevent {
    pub original: TimeSpec,
    pub event: IcalVevent,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IcalVeventSeries {
    pub master: IcalVevent,
    pub detached: Vec<IcalBoundDetachedVevent>,
}

/// Bind detached VEVENTs to the original recurrence slots of one master.
///
/// This does not flatten occurrence-specific semantics. It retains every typed
/// VEVENT and adds only the canonical original-slot identity needed for later
/// conversion.
pub fn bind_vevent_series(
    master: IcalVevent,
    detached: Vec<IcalVevent>,
) -> Result<IcalVeventSeries, IcalRecurrenceError> {
    if master.recurrence_id.is_some() {
        return Err(IcalRecurrenceError::MasterHasRecurrenceId);
    }
    if master.rrule.is_none() {
        return Err(IcalRecurrenceError::UnsupportedRecurrenceSet(
            "canonical series assembly currently requires RRULE; RDATE-only recurrence is not representable"
                .to_string(),
        ));
    }

    let mut bound = Vec::with_capacity(detached.len());
    for event in detached {
        if event.uid.value != master.uid.value {
            return Err(IcalRecurrenceError::UidMismatch {
                expected: master.uid.value.clone(),
                actual: event.uid.value.clone(),
            });
        }
        let recurrence_id = event
            .recurrence_id
            .as_ref()
            .ok_or(IcalRecurrenceError::DetachedMissingRecurrenceId)?;
        let raw = format_ical_content_line(recurrence_id)?;
        let original = parse_recurrence_id_property(&raw, &master.time)?;
        bound.push(IcalBoundDetachedVevent { original, event });
    }

    Ok(IcalVeventSeries {
        master,
        detached: bound,
    })
}

impl IcalVeventSeries {
    /// Project the full representable master/detached series into one canonical
    /// TemporalEvent while retaining normalized source components in metadata.
    pub fn canonical_event(&self) -> Result<TemporalEvent, IcalRecurrenceError> {
        let mut event = self.master.canonical_event()?;
        event.recurrence = Some(self.canonical_recurrence_rule()?);

        let detached_components = self
            .detached
            .iter()
            .map(|detached| format_vevent(&detached.event))
            .collect::<Result<Vec<_>, _>>()?;
        event.properties = json!({
            "ical": {
                "transport": "rfc5545_vevent",
                "uid": self.master.uid.value,
                "dtstamp": self.master.dtstamp.value.to_rfc3339(),
                "sequence": self.master.sequence.as_ref().map(|property| property.value),
                "master_component": format_vevent(&self.master)?,
                "detached_components": detached_components,
            }
        });
        event.validate_recurrence()?;
        Ok(event)
    }

    /// Convert the representable recurrence subset into the canonical domain
    /// rule. Unsupported occurrence-specific property changes are rejected
    /// instead of being discarded.
    pub fn canonical_recurrence_rule(&self) -> Result<RecurrenceRule, IcalRecurrenceError> {
        let mut rule = self
            .master
            .rrule
            .as_ref()
            .ok_or_else(|| {
                IcalRecurrenceError::UnsupportedRecurrenceSet(
                    "canonical series assembly requires RRULE".to_string(),
                )
            })?
            .value
            .clone();
        rule.rdates = self.master.rdates.clone();
        rule.exdates = self.master.exdates.clone();
        rule.overrides.clear();

        for detached in &self.detached {
            ensure_detached_temporal_override_is_representable(&self.master, &detached.event)?;

            let cancelled = detached
                .event
                .status
                .as_ref()
                .is_some_and(|status| status.value == IcalVeventStatus::Cancelled);
            let replacement = if detached.event.time == detached.original {
                None
            } else {
                Some(detached.event.time.clone())
            };
            rule.overrides.push(RecurrenceOverride {
                original: detached.original.clone(),
                replacement,
                cancelled,
            });
        }

        let mut event =
            TemporalEvent::new("iCalendar recurrence validation", self.master.time.clone());
        event.recurrence = Some(rule.clone());
        event.validate_recurrence()?;
        Ok(rule)
    }
}

fn ensure_detached_temporal_override_is_representable(
    master: &IcalVevent,
    detached: &IcalVevent,
) -> Result<(), IcalRecurrenceError> {
    if detached.rrule.is_some() || !detached.rdates.is_empty() || !detached.exdates.is_empty() {
        return Err(IcalRecurrenceError::UnsupportedDetachedOverride(
            "detached recurrence-set properties are not canonical occurrence overrides".to_string(),
        ));
    }

    if detached.summary.as_ref().map(|value| &value.value)
        != master.summary.as_ref().map(|value| &value.value)
        && detached.summary.is_some()
    {
        return Err(IcalRecurrenceError::UnsupportedDetachedOverride(
            "occurrence-specific SUMMARY".to_string(),
        ));
    }
    if detached.description.as_ref().map(|value| &value.value)
        != master.description.as_ref().map(|value| &value.value)
        && detached.description.is_some()
    {
        return Err(IcalRecurrenceError::UnsupportedDetachedOverride(
            "occurrence-specific DESCRIPTION".to_string(),
        ));
    }
    if !detached.extra_properties.is_empty() {
        return Err(IcalRecurrenceError::UnsupportedDetachedOverride(
            "occurrence-specific unmodeled properties".to_string(),
        ));
    }

    if let Some(status) = detached.status.as_ref() {
        let master_status = master.status.as_ref().map(|value| value.value);
        if status.value != IcalVeventStatus::Cancelled && master_status != Some(status.value) {
            return Err(IcalRecurrenceError::UnsupportedDetachedOverride(
                "occurrence-specific non-cancelled STATUS".to_string(),
            ));
        }
    }

    Ok(())
}

fn set_unique_content_line(
    slot: &mut Option<IcalContentLine>,
    line: IcalContentLine,
) -> Result<(), IcalRecurrenceError> {
    if slot.is_some() {
        return Err(IcalRecurrenceError::DuplicateProperty(line.name));
    }
    *slot = Some(line);
    Ok(())
}

fn parse_text_property(line: IcalContentLine) -> Result<IcalProperty<String>, IcalRecurrenceError> {
    Ok(IcalProperty {
        value: unescape_ical_text(&line.value)?,
        parameters: line.parameters,
    })
}

fn format_text_property_line(name: &str, property: &IcalProperty<String>) -> IcalContentLine {
    IcalContentLine {
        name: name.to_string(),
        parameters: property.parameters.clone(),
        value: escape_ical_text(&property.value),
    }
}

fn parse_dtstamp_property(
    line: IcalContentLine,
) -> Result<IcalProperty<DateTime<Utc>>, IcalRecurrenceError> {
    let Some(raw_utc) = line.value.strip_suffix('Z') else {
        return Err(IcalRecurrenceError::InvalidPropertyValue {
            property: "DTSTAMP",
            value: line.value,
        });
    };
    let naive = parse_basic_datetime("DTSTAMP", raw_utc)?;
    Ok(IcalProperty {
        value: DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc),
        parameters: line.parameters,
    })
}

fn parse_status_property(
    line: IcalContentLine,
) -> Result<IcalProperty<IcalVeventStatus>, IcalRecurrenceError> {
    let value = IcalVeventStatus::parse(&line.value).ok_or_else(|| {
        IcalRecurrenceError::InvalidPropertyValue {
            property: "STATUS",
            value: line.value.clone(),
        }
    })?;
    Ok(IcalProperty {
        value,
        parameters: line.parameters,
    })
}

fn parse_sequence_property(
    line: IcalContentLine,
) -> Result<IcalProperty<u32>, IcalRecurrenceError> {
    let signed =
        line.value
            .parse::<i32>()
            .map_err(|_| IcalRecurrenceError::InvalidPropertyValue {
                property: "SEQUENCE",
                value: line.value.clone(),
            })?;
    let value = u32::try_from(signed).map_err(|_| IcalRecurrenceError::InvalidPropertyValue {
        property: "SEQUENCE",
        value: line.value.clone(),
    })?;
    Ok(IcalProperty {
        value,
        parameters: line.parameters,
    })
}

fn parse_rrule_property(
    line: IcalContentLine,
) -> Result<IcalProperty<RecurrenceRule>, IcalRecurrenceError> {
    Ok(IcalProperty {
        value: parse_rrule(&line.value)?,
        parameters: line.parameters,
    })
}

fn is_reserved_typed_vevent_property(name: &str) -> bool {
    matches!(
        name.to_ascii_uppercase().as_str(),
        "UID"
            | "DTSTAMP"
            | "DTSTART"
            | "DTEND"
            | "DURATION"
            | "SUMMARY"
            | "DESCRIPTION"
            | "STATUS"
            | "SEQUENCE"
            | "RRULE"
            | "RDATE"
            | "EXDATE"
            | "RECURRENCE-ID"
    )
}

fn find_unquoted(raw: &str, needle: char) -> Option<usize> {
    let mut quoted = false;
    for (index, character) in raw.char_indices() {
        match character {
            '"' => quoted = !quoted,
            _ if character == needle && !quoted => return Some(index),
            _ => {}
        }
    }
    None
}

fn split_unquoted(raw: &str, separator: char) -> Result<Vec<&str>, IcalRecurrenceError> {
    let mut parts = Vec::new();
    let mut quoted = false;
    let mut start = 0_usize;
    for (index, character) in raw.char_indices() {
        match character {
            '"' => quoted = !quoted,
            _ if character == separator && !quoted => {
                parts.push(&raw[start..index]);
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }
    if quoted {
        return Err(IcalRecurrenceError::InvalidContentLine(raw.to_string()));
    }
    parts.push(&raw[start..]);
    Ok(parts)
}

fn validate_ical_token(token: &str, raw: &str) -> Result<(), IcalRecurrenceError> {
    if token.is_empty()
        || !token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(IcalRecurrenceError::InvalidContentLine(raw.to_string()));
    }
    Ok(())
}

fn valid_parameter_quoting(value: &str) -> bool {
    let starts = value.starts_with('"');
    let ends = value.ends_with('"');
    if starts != ends {
        return false;
    }
    if starts {
        value.len() >= 2 && !value[1..value.len() - 1].contains('"')
    } else {
        !value.contains('"')
    }
}

fn utf8_prefix_len(raw: &str, max_bytes: usize) -> usize {
    if raw.len() <= max_bytes {
        return raw.len();
    }
    let mut cut = max_bytes.min(raw.len());
    while cut > 0 && !raw.is_char_boundary(cut) {
        cut -= 1;
    }
    cut
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

/// Parse an RFC 5545 RECURRENCE-ID property into Ephemeris' canonical
/// original-slot identity. RANGE=THISANDFUTURE is rejected because the
/// current domain models single-instance overrides only.
pub fn parse_recurrence_id_property(
    raw: &str,
    base: &TimeSpec,
) -> Result<TimeSpec, IcalRecurrenceError> {
    let raw = raw.trim();
    let (head, raw_value) = raw
        .split_once(':')
        .ok_or_else(|| IcalRecurrenceError::InvalidProperty(raw.to_string()))?;
    if raw_value.trim().is_empty() {
        return Err(IcalRecurrenceError::EmptyPropertyValues("RECURRENCE-ID"));
    }
    if raw_value.contains(',') {
        return Err(IcalRecurrenceError::RecurrenceIdRequiresSingleValue);
    }

    let mut head_parts = head.split(';');
    let actual_name = head_parts
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_uppercase();
    if actual_name != "RECURRENCE-ID" {
        return Err(IcalRecurrenceError::WrongProperty {
            expected: "RECURRENCE-ID",
            actual: actual_name,
        });
    }

    let mut seen = HashSet::new();
    let mut tzid = None;
    let mut value_type = None;
    for raw_parameter in head_parts {
        let (raw_name, raw_parameter_value) = raw_parameter
            .split_once('=')
            .ok_or_else(|| IcalRecurrenceError::InvalidProperty(raw.to_string()))?;
        let name = raw_name.trim().to_ascii_uppercase();
        if !seen.insert(name.clone()) {
            return Err(IcalRecurrenceError::DuplicateParameter(name));
        }
        let value = unquote_parameter_value(raw_parameter_value.trim());
        if value.is_empty() {
            return Err(IcalRecurrenceError::InvalidProperty(raw.to_string()));
        }
        match name.as_str() {
            "VALUE" => value_type = Some(value.to_ascii_uppercase()),
            "TZID" => tzid = Some(value.to_string()),
            "RANGE" => {
                return Err(IcalRecurrenceError::UnsupportedRecurrenceRange(
                    value.to_ascii_uppercase(),
                ));
            }
            _ => return Err(IcalRecurrenceError::UnsupportedParameter(name)),
        }
    }

    let value_type = value_type.as_deref().unwrap_or("DATE-TIME");
    if !matches!(value_type, "DATE" | "DATE-TIME") {
        return Err(IcalRecurrenceError::UnsupportedValueType(
            value_type.to_string(),
        ));
    }

    match base {
        TimeSpec::Instant {
            source_timezone: Some(expected_timezone),
            ..
        } if tzid.as_deref() != Some(expected_timezone.as_str()) || raw_value.ends_with('Z') => {
            return Err(IcalRecurrenceError::TemporalKindMismatch {
                property: "RECURRENCE-ID",
                expected: base.kind_name(),
                actual: "non-source-local date-time",
            });
        }
        TimeSpec::Instant {
            source_timezone: None,
            ..
        } if tzid.is_some() || !raw_value.ends_with('Z') => {
            return Err(IcalRecurrenceError::TemporalKindMismatch {
                property: "RECURRENCE-ID",
                expected: base.kind_name(),
                actual: "non-UTC date-time",
            });
        }
        _ => {}
    }

    parse_recurrence_date_value(
        "RECURRENCE-ID",
        raw_value.trim(),
        value_type,
        tzid.as_deref(),
        base,
    )
}

/// Serialize one canonical original recurrence slot as RFC 5545 RECURRENCE-ID.
pub fn format_recurrence_id_property(
    original: &TimeSpec,
    base: &TimeSpec,
) -> Result<String, IcalRecurrenceError> {
    ensure_supported_exception_base(base)?;
    ensure_exception_shape("RECURRENCE-ID", original, base)?;

    match (base, original) {
        (TimeSpec::DateOnly { .. } | TimeSpec::AllDay { .. }, _) => Ok(format!(
            "RECURRENCE-ID;VALUE=DATE:{}",
            format_recurrence_date_value("RECURRENCE-ID", original)?
        )),
        (TimeSpec::Floating { .. }, _) => Ok(format!(
            "RECURRENCE-ID:{}",
            format_recurrence_date_value("RECURRENCE-ID", original)?
        )),
        (
            TimeSpec::Instant {
                source_timezone: Some(source_timezone),
                ..
            },
            TimeSpec::Instant { start_utc, .. },
        ) => {
            reject_fractional_seconds("RECURRENCE-ID", start_utc.naive_utc())?;
            let timezone = source_timezone
                .parse::<Tz>()
                .map_err(|_| IcalRecurrenceError::InvalidTimezone(source_timezone.clone()))?;
            let local = start_utc.with_timezone(&timezone);
            Ok(format!(
                "RECURRENCE-ID;TZID={source_timezone}:{}",
                local.format("%Y%m%dT%H%M%S")
            ))
        }
        (TimeSpec::Instant { .. }, _) => Ok(format!(
            "RECURRENCE-ID:{}",
            format_recurrence_date_value("RECURRENCE-ID", original)?
        )),
        (TimeSpec::Month { .. } | TimeSpec::Year { .. } | TimeSpec::Unknown { .. }, _) => {
            Err(IcalRecurrenceError::UnsupportedBaseKind(base.kind_name()))
        }
    }
}

/// Parse VEVENT DTSTART and optional DTEND into the canonical event time.
///
/// DATE values become explicit all-day events. Ephemeris DateOnly values are
/// intentionally not produced here because an RFC 5545 VEVENT DATE asserts
/// all-day/date semantics rather than merely imprecise civil-date precision.
pub fn parse_vevent_time_properties(
    dtstart: &str,
    dtend: Option<&str>,
) -> Result<TimeSpec, IcalRecurrenceError> {
    let start_property = parse_date_property(dtstart, "DTSTART")?;
    if start_property.values.len() != 1 {
        return Err(IcalRecurrenceError::EventTimeRequiresSingleValue("DTSTART"));
    }
    let start_value = start_property.values[0];
    let start_value_type = start_property.value_type.as_deref().unwrap_or("DATE-TIME");
    if !matches!(start_value_type, "DATE" | "DATE-TIME") {
        return Err(IcalRecurrenceError::UnsupportedValueType(
            start_value_type.to_string(),
        ));
    }

    if start_value_type == "DATE" {
        if start_property.tzid.is_some() {
            return Err(IcalRecurrenceError::UnsupportedParameter(
                "TZID".to_string(),
            ));
        }
        let start = NaiveDate::parse_from_str(start_value, "%Y%m%d").map_err(|_| {
            IcalRecurrenceError::InvalidPropertyValue {
                property: "DTSTART",
                value: start_value.to_string(),
            }
        })?;
        let end_exclusive = match dtend {
            Some(raw_end) => {
                let end_property = parse_date_property(raw_end, "DTEND")?;
                if end_property.values.len() != 1 {
                    return Err(IcalRecurrenceError::EventTimeRequiresSingleValue("DTEND"));
                }
                let end_value_type = end_property.value_type.as_deref().unwrap_or("DATE-TIME");
                if end_value_type != "DATE" {
                    return Err(IcalRecurrenceError::TemporalKindMismatch {
                        property: "DTEND",
                        expected: "date",
                        actual: value_type_kind(end_value_type),
                    });
                }
                if end_property.tzid.is_some() {
                    return Err(IcalRecurrenceError::UnsupportedParameter(
                        "TZID".to_string(),
                    ));
                }
                let raw_value = end_property.values[0];
                let end = NaiveDate::parse_from_str(raw_value, "%Y%m%d").map_err(|_| {
                    IcalRecurrenceError::InvalidPropertyValue {
                        property: "DTEND",
                        value: raw_value.to_string(),
                    }
                })?;
                if end <= start {
                    return Err(IcalRecurrenceError::InvalidPropertyValue {
                        property: "DTEND",
                        value: raw_value.to_string(),
                    });
                }
                Some(end)
            }
            None => None,
        };
        return Ok(TimeSpec::AllDay {
            start,
            end_exclusive,
        });
    }

    let start_time =
        parse_vevent_datetime_value("DTSTART", start_value, start_property.tzid.as_deref())?;
    let end_time = match dtend {
        Some(raw_end) => {
            let end_property = parse_date_property(raw_end, "DTEND")?;
            if end_property.values.len() != 1 {
                return Err(IcalRecurrenceError::EventTimeRequiresSingleValue("DTEND"));
            }
            let end_value_type = end_property.value_type.as_deref().unwrap_or("DATE-TIME");
            if end_value_type != "DATE-TIME" {
                return Err(IcalRecurrenceError::TemporalKindMismatch {
                    property: "DTEND",
                    expected: "date-time",
                    actual: value_type_kind(end_value_type),
                });
            }
            Some(parse_vevent_datetime_value(
                "DTEND",
                end_property.values[0],
                end_property.tzid.as_deref(),
            )?)
        }
        None => None,
    };

    match start_time {
        ParsedVeventDateTime::Floating(start) => {
            let end = match end_time {
                Some(ParsedVeventDateTime::Floating(end)) => {
                    if end <= start {
                        return Err(IcalRecurrenceError::InvalidPropertyValue {
                            property: "DTEND",
                            value: end.to_string(),
                        });
                    }
                    Some(end)
                }
                Some(other) => {
                    return Err(IcalRecurrenceError::TemporalKindMismatch {
                        property: "DTEND",
                        expected: "floating date-time",
                        actual: other.kind_name(),
                    });
                }
                None => None,
            };
            Ok(TimeSpec::Floating {
                start,
                end,
                source_timezone: None,
            })
        }
        ParsedVeventDateTime::Utc(start_utc) => {
            let end_utc = match end_time {
                Some(ParsedVeventDateTime::Utc(end_utc)) => {
                    if end_utc <= start_utc {
                        return Err(IcalRecurrenceError::InvalidPropertyValue {
                            property: "DTEND",
                            value: end_utc.to_rfc3339(),
                        });
                    }
                    Some(end_utc)
                }
                Some(other) => {
                    return Err(IcalRecurrenceError::TemporalKindMismatch {
                        property: "DTEND",
                        expected: "UTC date-time",
                        actual: other.kind_name(),
                    });
                }
                None => None,
            };
            Ok(TimeSpec::Instant {
                start_utc,
                end_utc,
                source_timezone: None,
            })
        }
        ParsedVeventDateTime::Zoned {
            utc: start_utc,
            timezone,
        } => {
            let end_utc = match end_time {
                Some(ParsedVeventDateTime::Zoned {
                    utc: end_utc,
                    timezone: end_timezone,
                }) if end_timezone == timezone => {
                    if end_utc <= start_utc {
                        return Err(IcalRecurrenceError::InvalidPropertyValue {
                            property: "DTEND",
                            value: end_utc.to_rfc3339(),
                        });
                    }
                    Some(end_utc)
                }
                Some(ParsedVeventDateTime::Zoned {
                    timezone: end_timezone,
                    ..
                }) => {
                    return Err(IcalRecurrenceError::TimezoneMismatch {
                        expected: Some(timezone),
                        actual: end_timezone,
                    });
                }
                Some(other) => {
                    return Err(IcalRecurrenceError::TemporalKindMismatch {
                        property: "DTEND",
                        expected: "zoned local date-time",
                        actual: other.kind_name(),
                    });
                }
                None => None,
            };
            Ok(TimeSpec::Instant {
                start_utc,
                end_utc,
                source_timezone: Some(timezone),
            })
        }
    }
}

/// Serialize a canonical event time as VEVENT DTSTART and optional DTEND.
///
/// DateOnly is rejected: emitting it as VALUE=DATE would strengthen uncertain
/// civil-date precision into all-day semantics.
pub fn format_vevent_time_properties(
    time: &TimeSpec,
) -> Result<(String, Option<String>), IcalRecurrenceError> {
    match time {
        TimeSpec::AllDay {
            start,
            end_exclusive,
        } => {
            let end = match end_exclusive {
                Some(end) if end <= start => {
                    return Err(IcalRecurrenceError::InvalidPropertyValue {
                        property: "DTEND",
                        value: end.to_string(),
                    });
                }
                Some(end) => Some(format!("DTEND;VALUE=DATE:{}", end.format("%Y%m%d"))),
                None => None,
            };
            Ok((
                format!("DTSTART;VALUE=DATE:{}", start.format("%Y%m%d")),
                end,
            ))
        }
        TimeSpec::DateOnly { .. } => Err(IcalRecurrenceError::UnsupportedVeventTimeKind(
            "date-only precision without explicit all-day semantics",
        )),
        TimeSpec::Floating {
            start,
            end,
            source_timezone,
        } => {
            if source_timezone.is_some() {
                return Err(IcalRecurrenceError::UnsupportedVeventTimeKind(
                    "floating date-time with source-timezone metadata",
                ));
            }
            reject_fractional_seconds("DTSTART", *start)?;
            let encoded_end = match end {
                Some(end) if end <= start => {
                    return Err(IcalRecurrenceError::InvalidPropertyValue {
                        property: "DTEND",
                        value: end.to_string(),
                    });
                }
                Some(end) => {
                    reject_fractional_seconds("DTEND", *end)?;
                    Some(format!("DTEND:{}", end.format("%Y%m%dT%H%M%S")))
                }
                None => None,
            };
            Ok((
                format!("DTSTART:{}", start.format("%Y%m%dT%H%M%S")),
                encoded_end,
            ))
        }
        TimeSpec::Instant {
            start_utc,
            end_utc,
            source_timezone,
        } => {
            reject_fractional_seconds("DTSTART", start_utc.naive_utc())?;
            if let Some(end) = end_utc
                && end <= start_utc
            {
                return Err(IcalRecurrenceError::InvalidPropertyValue {
                    property: "DTEND",
                    value: end.to_rfc3339(),
                });
            }
            match source_timezone {
                Some(raw_timezone) => {
                    let timezone = raw_timezone
                        .parse::<Tz>()
                        .map_err(|_| IcalRecurrenceError::InvalidTimezone(raw_timezone.clone()))?;
                    let local_start = start_utc.with_timezone(&timezone);
                    let encoded_end = match end_utc {
                        Some(end) => {
                            reject_fractional_seconds("DTEND", end.naive_utc())?;
                            let local_end = end.with_timezone(&timezone);
                            Some(format!(
                                "DTEND;TZID={raw_timezone}:{}",
                                local_end.format("%Y%m%dT%H%M%S")
                            ))
                        }
                        None => None,
                    };
                    Ok((
                        format!(
                            "DTSTART;TZID={raw_timezone}:{}",
                            local_start.format("%Y%m%dT%H%M%S")
                        ),
                        encoded_end,
                    ))
                }
                None => {
                    let encoded_end = match end_utc {
                        Some(end) => {
                            reject_fractional_seconds("DTEND", end.naive_utc())?;
                            Some(format!("DTEND:{}Z", end.format("%Y%m%dT%H%M%S")))
                        }
                        None => None,
                    };
                    Ok((
                        format!("DTSTART:{}Z", start_utc.format("%Y%m%dT%H%M%S")),
                        encoded_end,
                    ))
                }
            }
        }
        TimeSpec::Month { .. } | TimeSpec::Year { .. } | TimeSpec::Unknown { .. } => Err(
            IcalRecurrenceError::UnsupportedVeventTimeKind(time.kind_name()),
        ),
    }
}

#[derive(Debug)]
enum ParsedVeventDateTime {
    Floating(NaiveDateTime),
    Utc(DateTime<Utc>),
    Zoned {
        utc: DateTime<Utc>,
        timezone: String,
    },
}

impl ParsedVeventDateTime {
    const fn kind_name(&self) -> &'static str {
        match self {
            Self::Floating(_) => "floating date-time",
            Self::Utc(_) => "UTC date-time",
            Self::Zoned { .. } => "zoned local date-time",
        }
    }
}

fn parse_vevent_datetime_value(
    property: &'static str,
    raw: &str,
    tzid: Option<&str>,
) -> Result<ParsedVeventDateTime, IcalRecurrenceError> {
    if let Some(raw_timezone) = tzid {
        if raw.ends_with('Z') {
            return Err(IcalRecurrenceError::InvalidPropertyValue {
                property,
                value: raw.to_string(),
            });
        }
        let timezone = raw_timezone
            .parse::<Tz>()
            .map_err(|_| IcalRecurrenceError::InvalidTimezone(raw_timezone.to_string()))?;
        let local = parse_basic_datetime(property, raw)?;
        let utc = resolve_ical_local_datetime(property, timezone, local)?;
        return Ok(ParsedVeventDateTime::Zoned {
            utc,
            timezone: raw_timezone.to_string(),
        });
    }

    if let Some(utc_raw) = raw.strip_suffix('Z') {
        let utc = parse_basic_datetime(property, utc_raw)?;
        return Ok(ParsedVeventDateTime::Utc(
            DateTime::<Utc>::from_naive_utc_and_offset(utc, Utc),
        ));
    }

    Ok(ParsedVeventDateTime::Floating(parse_basic_datetime(
        property, raw,
    )?))
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

fn value_type_kind(value_type: &str) -> &'static str {
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
                        let value = if first_utc <= second_utc {
                            first
                        } else {
                            second
                        };
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
        ) => optional_duration(*base_start, *base_end) == optional_duration(*start, *end_exclusive),
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
    fn content_line_parser_handles_quoted_parameter_colons() {
        let line = parse_ical_content_line(
            "DESCRIPTION;ALTREP=\"cid:part1.0001@example.org\":The Fall'98 Wild Wizards",
        )
        .expect("content line");
        assert_eq!(line.name, "DESCRIPTION");
        assert_eq!(
            line.parameters,
            vec![(
                "ALTREP".to_string(),
                "\"cid:part1.0001@example.org\"".to_string()
            )]
        );
        assert_eq!(line.value, "The Fall'98 Wild Wizards");
        assert_eq!(
            format_ical_content_line(&line).expect("format"),
            "DESCRIPTION;ALTREP=\"cid:part1.0001@example.org\":The Fall'98 Wild Wizards"
        );
    }

    #[test]
    fn content_lines_unfold_rfc_continuations() {
        let raw = "DESCRIPTION:This is a long description that exists on a long line.\r\n \
and continues here\r\nSUMMARY:Example\r\n";
        assert_eq!(
            unfold_ical_content_lines(raw).expect("unfold"),
            vec![
                "DESCRIPTION:This is a long description that exists on a long line.and continues here",
                "SUMMARY:Example",
            ]
        );
    }

    #[test]
    fn ical_text_escape_roundtrips_delimiters_and_newlines() {
        let original = "alpha\\beta; gamma, delta: epsilon\nsecond line";
        let encoded = escape_ical_text(original);
        assert_eq!(
            encoded,
            "alpha\\\\beta\\; gamma\\, delta: epsilon\\nsecond line"
        );
        assert_eq!(unescape_ical_text(&encoded).expect("unescape"), original);
        assert!(matches!(
            unescape_ical_text("bad\\qescape"),
            Err(IcalRecurrenceError::InvalidTextEscape(_))
        ));
    }

    #[test]
    fn content_line_folding_is_utf8_safe_and_reversible() {
        let logical = format!("DESCRIPTION:{}{}", "é".repeat(40), "x".repeat(30));
        let folded = fold_ical_content_line(&logical).expect("fold");
        for physical in folded.split("\r\n") {
            assert!(physical.len() <= 75);
        }
        assert_eq!(
            unfold_ical_content_lines(&format!("{folded}\r\n")).expect("unfold"),
            vec![logical]
        );
    }

    #[test]
    fn vevent_envelope_preserves_unknown_and_parameterized_properties() {
        let raw = concat!(
            "BEGIN:VEVENT\r\n",
            "UID:abc-123\r\n",
            "SUMMARY;LANGUAGE=en-US:Hello\\, world\r\n",
            "X-EPHEMERIS-TEST:opaque:value\r\n",
            "END:VEVENT\r\n"
        );
        let lines = parse_vevent_content_lines(raw).expect("VEVENT");
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[1].name, "SUMMARY");
        assert_eq!(
            lines[1].parameters,
            vec![("LANGUAGE".to_string(), "en-US".to_string())]
        );
        assert_eq!(lines[2].value, "opaque:value");
        assert_eq!(
            parse_vevent_content_lines(&format_vevent_content_lines(&lines).expect("format"))
                .expect("reparse"),
            lines
        );
    }

    #[test]
    fn vevent_envelope_rejects_nested_components_and_bad_continuations() {
        assert!(matches!(
            parse_vevent_content_lines(
                "BEGIN:VEVENT\r\nBEGIN:VALARM\r\nEND:VALARM\r\nEND:VEVENT\r\n"
            ),
            Err(IcalRecurrenceError::InvalidComponent(_))
        ));
        assert!(matches!(
            unfold_ical_content_lines(" orphan continuation\r\n"),
            Err(IcalRecurrenceError::InvalidContentLine(_))
        ));
    }

    #[test]
    fn typed_vevent_roundtrips_recurrence_text_and_unknown_properties() {
        let raw = concat!(
            "BEGIN:VEVENT\r\n",
            "UID:series-123@example.com\r\n",
            "DTSTAMP:20261006T120000Z\r\n",
            "DTSTART;TZID=America/New_York:20261102T090000\r\n",
            "DTEND;TZID=America/New_York:20261102T100000\r\n",
            "RRULE:FREQ=WEEKLY;BYDAY=MO,WE\r\n",
            "RDATE;TZID=America/New_York:20261106T090000\r\n",
            "EXDATE;TZID=America/New_York:20261109T090000\r\n",
            "SUMMARY;LANGUAGE=en:Team\\, sync\r\n",
            "DESCRIPTION:Line 1\\nLine 2\r\n",
            "STATUS:CONFIRMED\r\n",
            "SEQUENCE:2\r\n",
            "LOCATION:Room 3\r\n",
            "END:VEVENT\r\n"
        );
        let event = parse_vevent(raw).expect("typed VEVENT");
        assert_eq!(event.uid.value, "series-123@example.com");
        assert_eq!(
            event
                .summary
                .as_ref()
                .map(|property| property.value.as_str()),
            Some("Team, sync")
        );
        assert_eq!(
            event
                .description
                .as_ref()
                .map(|property| property.value.as_str()),
            Some("Line 1\nLine 2")
        );
        assert_eq!(
            event.status.as_ref().map(|property| property.value),
            Some(IcalVeventStatus::Confirmed)
        );
        assert_eq!(
            event.sequence.as_ref().map(|property| property.value),
            Some(2)
        );
        assert_eq!(event.rdates.len(), 1);
        assert_eq!(event.exdates.len(), 1);
        assert_eq!(event.extra_properties.len(), 1);
        assert_eq!(event.extra_properties[0].name, "LOCATION");

        let reparsed = parse_vevent(&format_vevent(&event).expect("format")).expect("reparse");
        assert_eq!(reparsed, event);
    }

    #[test]
    fn typed_vevent_preserves_recurrence_id_for_master_context_binding() {
        let raw = concat!(
            "BEGIN:VEVENT\r\n",
            "UID:series-123@example.com\r\n",
            "DTSTAMP:20261006T120000Z\r\n",
            "RECURRENCE-ID;TZID=America/New_York:20261109T090000\r\n",
            "DTSTART;TZID=America/New_York:20261109T110000\r\n",
            "DTEND;TZID=America/New_York:20261109T120000\r\n",
            "STATUS:CONFIRMED\r\n",
            "END:VEVENT\r\n"
        );
        let event = parse_vevent(raw).expect("detached VEVENT");
        let recurrence_id = event.recurrence_id.as_ref().expect("RECURRENCE-ID");
        assert_eq!(recurrence_id.name, "RECURRENCE-ID");
        assert_eq!(recurrence_id.value, "20261109T090000");
        assert_eq!(
            format_ical_content_line(recurrence_id).expect("format RECURRENCE-ID"),
            "RECURRENCE-ID;TZID=America/New_York:20261109T090000"
        );
        assert_eq!(
            parse_vevent(&format_vevent(&event).expect("format")).expect("reparse"),
            event
        );
    }

    #[test]
    fn typed_vevent_requires_uid_dtstamp_and_dtstart() {
        for (raw, property) in [
            (
                "BEGIN:VEVENT\r\nDTSTAMP:20261006T120000Z\r\nDTSTART:20261007T090000Z\r\nEND:VEVENT\r\n",
                "UID",
            ),
            (
                "BEGIN:VEVENT\r\nUID:a@example.com\r\nDTSTART:20261007T090000Z\r\nEND:VEVENT\r\n",
                "DTSTAMP",
            ),
            (
                "BEGIN:VEVENT\r\nUID:a@example.com\r\nDTSTAMP:20261006T120000Z\r\nEND:VEVENT\r\n",
                "DTSTART",
            ),
        ] {
            assert_eq!(
                parse_vevent(raw),
                Err(IcalRecurrenceError::MissingProperty(property))
            );
        }
    }

    #[test]
    fn typed_vevent_rejects_duplicate_singletons_and_duration() {
        assert!(matches!(
            parse_vevent(concat!(
                "BEGIN:VEVENT\r\n",
                "UID:a@example.com\r\n",
                "UID:b@example.com\r\n",
                "DTSTAMP:20261006T120000Z\r\n",
                "DTSTART:20261007T090000Z\r\n",
                "END:VEVENT\r\n"
            )),
            Err(IcalRecurrenceError::DuplicateProperty(name)) if name == "UID"
        ));

        assert_eq!(
            parse_vevent(concat!(
                "BEGIN:VEVENT\r\n",
                "UID:a@example.com\r\n",
                "DTSTAMP:20261006T120000Z\r\n",
                "DTSTART:20261007T090000Z\r\n",
                "DURATION:PT1H\r\n",
                "END:VEVENT\r\n"
            )),
            Err(IcalRecurrenceError::UnsupportedProperty(
                "DURATION".to_string()
            ))
        );
    }

    #[test]
    fn typed_vevent_rejects_non_utc_dtstamp_and_invalid_status_sequence() {
        let base = concat!(
            "BEGIN:VEVENT\r\n",
            "UID:a@example.com\r\n",
            "DTSTART:20261007T090000Z\r\n"
        );
        assert!(matches!(
            parse_vevent(&format!("{base}DTSTAMP:20261006T120000\r\nEND:VEVENT\r\n")),
            Err(IcalRecurrenceError::InvalidPropertyValue {
                property: "DTSTAMP",
                ..
            })
        ));
        assert!(matches!(
            parse_vevent(&format!(
                "{base}DTSTAMP:20261006T120000Z\r\nSTATUS:COMPLETED\r\nEND:VEVENT\r\n"
            )),
            Err(IcalRecurrenceError::InvalidPropertyValue {
                property: "STATUS",
                ..
            })
        ));
        assert!(matches!(
            parse_vevent(&format!(
                "{base}DTSTAMP:20261006T120000Z\r\nSEQUENCE:-1\r\nEND:VEVENT\r\n"
            )),
            Err(IcalRecurrenceError::InvalidPropertyValue {
                property: "SEQUENCE",
                ..
            })
        ));
    }

    #[test]
    fn canonical_standalone_event_exports_and_roundtrips_through_vcalendar() {
        let start = DateTime::parse_from_rfc3339("2026-10-07T14:00:00Z")
            .expect("start")
            .with_timezone(&Utc);
        let mut event = TemporalEvent::new(
            "Project review",
            TimeSpec::Instant {
                start_utc: start,
                end_utc: Some(start + Duration::hours(1)),
                source_timezone: None,
            },
        );
        event.description = Some("Decisions, next steps".to_string());
        event.status = EventStatus::Confirmed;
        event.updated_at = DateTime::parse_from_rfc3339("2026-10-06T12:34:56.987Z")
            .expect("updated")
            .with_timezone(&Utc);

        let encoded = format_temporal_events_vcalendar(&[event.clone()], "-//Ephemeris Test//EN")
            .expect("export");
        assert!(encoded.contains(&format!("UID:urn:uuid:{}", event.id)));
        assert!(encoded.contains("DTSTAMP:20261006T123456Z"));
        assert!(encoded.contains("SUMMARY:Project review"));
        assert!(encoded.contains("STATUS:CONFIRMED"));

        let imported = parse_vcalendar(&encoded)
            .expect("parse export")
            .canonical_events()
            .expect("canonical reimport");
        assert_eq!(imported.len(), 1);
        assert_eq!(imported[0].normalized_title, event.normalized_title);
        assert_eq!(imported[0].description, event.description);
        assert_eq!(imported[0].status, event.status);
        assert_eq!(imported[0].time, event.time);
    }

    #[test]
    fn canonical_recurrence_exports_detached_override_components_and_roundtrips() {
        let start = DateTime::parse_from_rfc3339("2026-11-02T09:00:00Z")
            .expect("start")
            .with_timezone(&Utc);
        let original = TimeSpec::Instant {
            start_utc: start + Duration::days(1),
            end_utc: Some(start + Duration::days(1) + Duration::hours(1)),
            source_timezone: None,
        };
        let replacement = TimeSpec::Instant {
            start_utc: start + Duration::days(1) + Duration::hours(2),
            end_utc: Some(start + Duration::days(1) + Duration::hours(3)),
            source_timezone: None,
        };
        let cancelled_original = TimeSpec::Instant {
            start_utc: start + Duration::days(2),
            end_utc: Some(start + Duration::days(2) + Duration::hours(1)),
            source_timezone: None,
        };
        let mut event = TemporalEvent::new(
            "Daily sync",
            TimeSpec::Instant {
                start_utc: start,
                end_utc: Some(start + Duration::hours(1)),
                source_timezone: None,
            },
        );
        event.updated_at = DateTime::parse_from_rfc3339("2026-10-06T12:00:00Z")
            .expect("updated")
            .with_timezone(&Utc);
        event.recurrence = Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Daily,
            count: Some(3),
            overrides: vec![
                RecurrenceOverride {
                    original: original.clone(),
                    replacement: Some(replacement.clone()),
                    cancelled: false,
                },
                RecurrenceOverride {
                    original: cancelled_original.clone(),
                    replacement: None,
                    cancelled: true,
                },
            ],
            ..RecurrenceRule::default()
        });
        event.validate_recurrence().expect("canonical recurrence");

        let calendar = export_temporal_events_vcalendar(&[event.clone()], "-//Ephemeris Test//EN")
            .expect("calendar");
        assert_eq!(calendar.events.len(), 3);
        assert!(calendar.events[0].recurrence_id.is_none());
        assert!(calendar.events[1].recurrence_id.is_some());
        assert_eq!(calendar.events[1].time, replacement);
        assert_eq!(
            calendar.events[2]
                .status
                .as_ref()
                .map(|status| status.value),
            Some(IcalVeventStatus::Cancelled)
        );

        let roundtrip = parse_vcalendar(&format_vcalendar(&calendar).expect("format"))
            .expect("parse")
            .canonical_events()
            .expect("canonical events");
        let recurrence = roundtrip[0].recurrence.as_ref().expect("recurrence");
        assert_eq!(
            recurrence.overrides,
            event.recurrence.as_ref().unwrap().overrides
        );
    }

    #[test]
    fn canonical_export_preserves_imported_unknown_master_properties_and_text_parameters() {
        let source = parse_vcalendar(concat!(
            "BEGIN:VCALENDAR\r\n",
            "PRODID:-//Source//EN\r\n",
            "VERSION:2.0\r\n",
            "BEGIN:VEVENT\r\n",
            "UID:preserve@example.com\r\n",
            "DTSTAMP:20261006T120000Z\r\n",
            "DTSTART:20261007T090000Z\r\n",
            "SUMMARY;LANGUAGE=es:Revisión\r\n",
            "LOCATION:Sala 4\r\n",
            "END:VEVENT\r\n",
            "END:VCALENDAR\r\n"
        ))
        .expect("source");
        let mut event = source
            .canonical_events()
            .expect("canonical")
            .pop()
            .expect("event");
        event.normalized_title = "Revisión final".to_string();

        let exported = export_temporal_event(&event).expect("export");
        assert_eq!(exported.len(), 1);
        assert_eq!(exported[0].uid.value, "preserve@example.com");
        assert_eq!(
            exported[0].summary.as_ref().expect("summary").parameters,
            vec![("LANGUAGE".to_string(), "es".to_string())]
        );
        assert_eq!(exported[0].extra_properties.len(), 1);
        assert_eq!(exported[0].extra_properties[0].name, "LOCATION");
        assert_eq!(
            exported[0].summary.as_ref().unwrap().value,
            "Revisión final"
        );
    }

    #[test]
    fn canonical_export_does_not_invent_summary_for_imported_untitled_event() {
        let source = parse_vevent(concat!(
            "BEGIN:VEVENT\r\n",
            "UID:untitled@example.com\r\n",
            "DTSTAMP:20261006T120000Z\r\n",
            "DTSTART;VALUE=DATE:20261007\r\n",
            "END:VEVENT\r\n"
        ))
        .expect("source");
        let event = source.canonical_event().expect("canonical");

        let exported = export_temporal_event(&event).expect("export");
        assert!(exported[0].summary.is_none());
        assert_eq!(exported[0].uid.value, "untitled@example.com");
    }

    #[test]
    fn canonical_export_rejects_non_rfc_status_and_imprecise_date_only_time() {
        let mut completed = TemporalEvent::new(
            "Completed",
            TimeSpec::AllDay {
                start: NaiveDate::from_ymd_opt(2026, 10, 7).expect("date"),
                end_exclusive: None,
            },
        );
        completed.status = EventStatus::Completed;
        assert_eq!(
            export_temporal_event(&completed),
            Err(IcalRecurrenceError::UnsupportedCanonicalStatus(
                "completed".to_string()
            ))
        );

        let date_only = TemporalEvent::new(
            "Imprecise date",
            TimeSpec::DateOnly {
                start: NaiveDate::from_ymd_opt(2026, 10, 7).expect("date"),
                end_exclusive: None,
            },
        );
        assert!(matches!(
            export_temporal_event(&date_only),
            Err(IcalRecurrenceError::UnsupportedVeventTimeKind(_))
        ));
    }

    #[test]
    fn canonical_export_rejects_empty_prodid() {
        let event = TemporalEvent::new(
            "Example",
            TimeSpec::AllDay {
                start: NaiveDate::from_ymd_opt(2026, 10, 7).expect("date"),
                end_exclusive: None,
            },
        );
        assert!(matches!(
            export_temporal_events_vcalendar(&[event], "   "),
            Err(IcalRecurrenceError::InvalidPropertyValue {
                property: "PRODID",
                ..
            })
        ));
    }

    #[test]
    fn vcalendar_groups_standalone_and_detached_series_into_canonical_events() {
        let raw = concat!(
            "BEGIN:VCALENDAR\r\n",
            "PRODID:-//Ephemeris Test//EN\r\n",
            "VERSION:2.0\r\n",
            "X-WR-CALNAME:Fixture\r\n",
            "BEGIN:VEVENT\r\n",
            "UID:standalone@example.com\r\n",
            "DTSTAMP:20261006T120000Z\r\n",
            "DTSTART;VALUE=DATE:20261010\r\n",
            "SUMMARY:Standalone\r\n",
            "END:VEVENT\r\n",
            "BEGIN:VEVENT\r\n",
            "UID:series@example.com\r\n",
            "DTSTAMP:20261006T120000Z\r\n",
            "DTSTART:20261102T090000Z\r\n",
            "DTEND:20261102T100000Z\r\n",
            "RRULE:FREQ=DAILY;COUNT=2\r\n",
            "SUMMARY:Series\r\n",
            "END:VEVENT\r\n",
            "BEGIN:VEVENT\r\n",
            "UID:series@example.com\r\n",
            "DTSTAMP:20261007T120000Z\r\n",
            "RECURRENCE-ID:20261103T090000Z\r\n",
            "DTSTART:20261103T110000Z\r\n",
            "DTEND:20261103T120000Z\r\n",
            "SUMMARY:Series\r\n",
            "END:VEVENT\r\n",
            "END:VCALENDAR\r\n"
        );

        let calendar = parse_vcalendar(raw).expect("VCALENDAR");
        assert_eq!(calendar.events.len(), 3);
        assert_eq!(calendar.properties.len(), 3);

        let events = calendar.canonical_events().expect("canonical events");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].normalized_title, "Standalone");
        assert!(events[0].recurrence.is_none());
        assert_eq!(events[1].normalized_title, "Series");
        assert_eq!(
            events[1]
                .recurrence
                .as_ref()
                .expect("series recurrence")
                .overrides
                .len(),
            1
        );
    }

    #[test]
    fn vcalendar_roundtrips_supported_top_level_properties_and_events() {
        let raw = concat!(
            "BEGIN:VCALENDAR\r\n",
            "PRODID:-//Ephemeris Test//EN\r\n",
            "VERSION:2.0\r\n",
            "CALSCALE:GREGORIAN\r\n",
            "BEGIN:VEVENT\r\n",
            "UID:a@example.com\r\n",
            "DTSTAMP:20261006T120000Z\r\n",
            "DTSTART:20261007T090000Z\r\n",
            "SUMMARY:Example\r\n",
            "END:VEVENT\r\n",
            "END:VCALENDAR\r\n"
        );
        let calendar = parse_vcalendar(raw).expect("parse");
        let reparsed =
            parse_vcalendar(&format_vcalendar(&calendar).expect("format")).expect("reparse");
        assert_eq!(reparsed, calendar);
    }

    #[test]
    fn vcalendar_requires_version_2_and_prodid() {
        assert_eq!(
            parse_vcalendar(concat!(
                "BEGIN:VCALENDAR\r\n",
                "PRODID:-//Ephemeris Test//EN\r\n",
                "END:VCALENDAR\r\n"
            )),
            Err(IcalRecurrenceError::MissingProperty("VERSION"))
        );
        assert_eq!(
            parse_vcalendar(concat!(
                "BEGIN:VCALENDAR\r\n",
                "VERSION:2.0\r\n",
                "END:VCALENDAR\r\n"
            )),
            Err(IcalRecurrenceError::MissingProperty("PRODID"))
        );
        assert!(matches!(
            parse_vcalendar(concat!(
                "BEGIN:VCALENDAR\r\n",
                "PRODID:-//Ephemeris Test//EN\r\n",
                "VERSION:1.0\r\n",
                "END:VCALENDAR\r\n"
            )),
            Err(IcalRecurrenceError::InvalidPropertyValue {
                property: "VERSION",
                ..
            })
        ));
    }

    #[test]
    fn vcalendar_rejects_unsupported_nested_component_families_explicitly() {
        assert_eq!(
            parse_vcalendar(concat!(
                "BEGIN:VCALENDAR\r\n",
                "PRODID:-//Ephemeris Test//EN\r\n",
                "VERSION:2.0\r\n",
                "BEGIN:VTIMEZONE\r\n",
                "TZID:Custom/Zone\r\n",
                "END:VTIMEZONE\r\n",
                "END:VCALENDAR\r\n"
            )),
            Err(IcalRecurrenceError::UnsupportedComponent(
                "VTIMEZONE".to_string()
            ))
        );

        assert_eq!(
            parse_vcalendar(concat!(
                "BEGIN:VCALENDAR\r\n",
                "PRODID:-//Ephemeris Test//EN\r\n",
                "VERSION:2.0\r\n",
                "BEGIN:VEVENT\r\n",
                "UID:a@example.com\r\n",
                "DTSTAMP:20261006T120000Z\r\n",
                "DTSTART:20261007T090000Z\r\n",
                "BEGIN:VALARM\r\n",
                "ACTION:DISPLAY\r\n",
                "END:VALARM\r\n",
                "END:VEVENT\r\n",
                "END:VCALENDAR\r\n"
            )),
            Err(IcalRecurrenceError::UnsupportedComponent(
                "VALARM".to_string()
            ))
        );
    }

    #[test]
    fn vcalendar_rejects_orphan_detached_and_duplicate_master_uid_groups() {
        let orphan = parse_vcalendar(concat!(
            "BEGIN:VCALENDAR\r\n",
            "PRODID:-//Ephemeris Test//EN\r\n",
            "VERSION:2.0\r\n",
            "BEGIN:VEVENT\r\n",
            "UID:series@example.com\r\n",
            "DTSTAMP:20261007T120000Z\r\n",
            "RECURRENCE-ID:20261103T090000Z\r\n",
            "DTSTART:20261103T110000Z\r\n",
            "END:VEVENT\r\n",
            "END:VCALENDAR\r\n"
        ))
        .expect("orphan calendar");
        assert_eq!(
            orphan.canonical_events(),
            Err(IcalRecurrenceError::MissingMaster(
                "series@example.com".to_string()
            ))
        );

        let duplicate = parse_vcalendar(concat!(
            "BEGIN:VCALENDAR\r\n",
            "PRODID:-//Ephemeris Test//EN\r\n",
            "VERSION:2.0\r\n",
            "BEGIN:VEVENT\r\n",
            "UID:duplicate@example.com\r\n",
            "DTSTAMP:20261006T120000Z\r\n",
            "DTSTART:20261007T090000Z\r\n",
            "END:VEVENT\r\n",
            "BEGIN:VEVENT\r\n",
            "UID:duplicate@example.com\r\n",
            "DTSTAMP:20261007T120000Z\r\n",
            "DTSTART:20261008T090000Z\r\n",
            "END:VEVENT\r\n",
            "END:VCALENDAR\r\n"
        ))
        .expect("duplicate calendar");
        assert_eq!(
            duplicate.canonical_events(),
            Err(IcalRecurrenceError::DuplicateMaster(
                "duplicate@example.com".to_string()
            ))
        );
    }

    #[test]
    fn standalone_vevent_projects_into_canonical_event_and_retains_transport() {
        let source = parse_vevent(concat!(
            "BEGIN:VEVENT\r\n",
            "UID:standalone@example.com\r\n",
            "DTSTAMP:20261006T120000Z\r\n",
            "DTSTART:20261007T140000Z\r\n",
            "DTEND:20261007T150000Z\r\n",
            "SUMMARY:Project review\r\n",
            "DESCRIPTION:Review\\, decisions\\nNext steps\r\n",
            "STATUS:TENTATIVE\r\n",
            "SEQUENCE:3\r\n",
            "LOCATION:Room 5\r\n",
            "END:VEVENT\r\n"
        ))
        .expect("VEVENT");
        let event = source.canonical_event().expect("canonical event");

        assert_eq!(
            event.source_record_key.as_deref(),
            Some("standalone@example.com")
        );
        assert_eq!(event.normalized_title, "Project review");
        assert_eq!(event.raw_title.as_deref(), Some("Project review"));
        assert_eq!(
            event.description.as_deref(),
            Some("Review, decisions\nNext steps")
        );
        assert_eq!(event.status, EventStatus::Tentative);
        assert!(event.recurrence.is_none());
        assert_eq!(
            event.properties["ical"]["uid"],
            json!("standalone@example.com")
        );
        assert_eq!(event.properties["ical"]["sequence"], json!(3));
        assert!(
            event.properties["ical"]["master_component"]
                .as_str()
                .expect("master component")
                .contains("LOCATION:Room 5")
        );
    }

    #[test]
    fn standalone_vevent_without_summary_uses_uid_as_noninvented_fallback_title() {
        let source = parse_vevent(concat!(
            "BEGIN:VEVENT\r\n",
            "UID:untitled@example.com\r\n",
            "DTSTAMP:20261006T120000Z\r\n",
            "DTSTART;VALUE=DATE:20261007\r\n",
            "END:VEVENT\r\n"
        ))
        .expect("VEVENT");
        let event = source.canonical_event().expect("canonical event");

        assert_eq!(event.normalized_title, "untitled@example.com");
        assert!(event.raw_title.is_none());
        assert!(matches!(event.time, TimeSpec::AllDay { .. }));
    }

    #[test]
    fn recurring_vevent_series_projects_overrides_and_retains_detached_components() {
        let master = parse_vevent(concat!(
            "BEGIN:VEVENT\r\n",
            "UID:projection-series@example.com\r\n",
            "DTSTAMP:20261006T120000Z\r\n",
            "DTSTART:20261102T090000Z\r\n",
            "DTEND:20261102T100000Z\r\n",
            "RRULE:FREQ=DAILY;COUNT=2\r\n",
            "SUMMARY:Daily sync\r\n",
            "END:VEVENT\r\n"
        ))
        .expect("master");
        let moved = parse_vevent(concat!(
            "BEGIN:VEVENT\r\n",
            "UID:projection-series@example.com\r\n",
            "DTSTAMP:20261007T120000Z\r\n",
            "RECURRENCE-ID:20261103T090000Z\r\n",
            "DTSTART:20261103T110000Z\r\n",
            "DTEND:20261103T120000Z\r\n",
            "SUMMARY:Daily sync\r\n",
            "END:VEVENT\r\n"
        ))
        .expect("moved");

        let series = bind_vevent_series(master, vec![moved]).expect("series");
        let event = series.canonical_event().expect("canonical event");

        assert_eq!(
            event.source_record_key.as_deref(),
            Some("projection-series@example.com")
        );
        let recurrence = event.recurrence.as_ref().expect("recurrence");
        assert_eq!(recurrence.overrides.len(), 1);
        assert!(recurrence.overrides[0].replacement.is_some());
        assert_eq!(
            event.properties["ical"]["detached_components"]
                .as_array()
                .expect("detached components")
                .len(),
            1
        );
    }

    #[test]
    fn canonical_projection_rejects_detached_directly_and_rdate_only_recurrence() {
        let detached = parse_vevent(concat!(
            "BEGIN:VEVENT\r\n",
            "UID:series@example.com\r\n",
            "DTSTAMP:20261007T120000Z\r\n",
            "RECURRENCE-ID:20261103T090000Z\r\n",
            "DTSTART:20261103T110000Z\r\n",
            "END:VEVENT\r\n"
        ))
        .expect("detached");
        assert_eq!(
            detached.canonical_event(),
            Err(IcalRecurrenceError::DetachedMissingRecurrenceId)
        );

        let rdate_only = parse_vevent(concat!(
            "BEGIN:VEVENT\r\n",
            "UID:rdate-only@example.com\r\n",
            "DTSTAMP:20261006T120000Z\r\n",
            "DTSTART:20261102T090000Z\r\n",
            "RDATE:20261103T090000Z\r\n",
            "END:VEVENT\r\n"
        ))
        .expect("RDATE-only");
        assert!(matches!(
            rdate_only.canonical_event(),
            Err(IcalRecurrenceError::UnsupportedRecurrenceSet(_))
        ));
    }

    #[test]
    fn vevent_series_assembles_moved_cancelled_and_cancelled_moved_overrides() {
        let master = parse_vevent(concat!(
            "BEGIN:VEVENT\r\n",
            "UID:series@example.com\r\n",
            "DTSTAMP:20261006T120000Z\r\n",
            "DTSTART;TZID=America/New_York:20261102T090000\r\n",
            "DTEND;TZID=America/New_York:20261102T100000\r\n",
            "RRULE:FREQ=WEEKLY;COUNT=4\r\n",
            "SUMMARY:Weekly sync\r\n",
            "STATUS:CONFIRMED\r\n",
            "END:VEVENT\r\n"
        ))
        .expect("master");

        let moved = parse_vevent(concat!(
            "BEGIN:VEVENT\r\n",
            "UID:series@example.com\r\n",
            "DTSTAMP:20261007T120000Z\r\n",
            "RECURRENCE-ID;TZID=America/New_York:20261109T090000\r\n",
            "DTSTART;TZID=America/New_York:20261109T110000\r\n",
            "DTEND;TZID=America/New_York:20261109T120000\r\n",
            "SUMMARY:Weekly sync\r\n",
            "STATUS:CONFIRMED\r\n",
            "END:VEVENT\r\n"
        ))
        .expect("moved");

        let cancelled = parse_vevent(concat!(
            "BEGIN:VEVENT\r\n",
            "UID:series@example.com\r\n",
            "DTSTAMP:20261008T120000Z\r\n",
            "RECURRENCE-ID;TZID=America/New_York:20261116T090000\r\n",
            "DTSTART;TZID=America/New_York:20261116T090000\r\n",
            "DTEND;TZID=America/New_York:20261116T100000\r\n",
            "STATUS:CANCELLED\r\n",
            "END:VEVENT\r\n"
        ))
        .expect("cancelled");

        let cancelled_moved = parse_vevent(concat!(
            "BEGIN:VEVENT\r\n",
            "UID:series@example.com\r\n",
            "DTSTAMP:20261009T120000Z\r\n",
            "RECURRENCE-ID;TZID=America/New_York:20261123T090000\r\n",
            "DTSTART;TZID=America/New_York:20261123T130000\r\n",
            "DTEND;TZID=America/New_York:20261123T140000\r\n",
            "STATUS:CANCELLED\r\n",
            "END:VEVENT\r\n"
        ))
        .expect("cancelled moved");

        let series =
            bind_vevent_series(master, vec![moved, cancelled, cancelled_moved]).expect("series");
        let rule = series.canonical_recurrence_rule().expect("canonical rule");
        assert_eq!(rule.overrides.len(), 3);

        assert!(!rule.overrides[0].cancelled);
        assert!(rule.overrides[0].replacement.is_some());
        assert!(rule.overrides[1].cancelled);
        assert!(rule.overrides[1].replacement.is_none());
        assert!(rule.overrides[2].cancelled);
        assert!(rule.overrides[2].replacement.is_some());
    }

    #[test]
    fn vevent_series_rejects_uid_mismatch_missing_identity_and_master_identity() {
        let master = parse_vevent(concat!(
            "BEGIN:VEVENT\r\n",
            "UID:series@example.com\r\n",
            "DTSTAMP:20261006T120000Z\r\n",
            "DTSTART:20261102T090000Z\r\n",
            "RRULE:FREQ=DAILY;COUNT=2\r\n",
            "END:VEVENT\r\n"
        ))
        .expect("master");

        let wrong_uid = parse_vevent(concat!(
            "BEGIN:VEVENT\r\n",
            "UID:other@example.com\r\n",
            "DTSTAMP:20261007T120000Z\r\n",
            "RECURRENCE-ID:20261103T090000Z\r\n",
            "DTSTART:20261103T100000Z\r\n",
            "END:VEVENT\r\n"
        ))
        .expect("wrong UID");
        assert!(matches!(
            bind_vevent_series(master.clone(), vec![wrong_uid]),
            Err(IcalRecurrenceError::UidMismatch { .. })
        ));

        let missing_identity = parse_vevent(concat!(
            "BEGIN:VEVENT\r\n",
            "UID:series@example.com\r\n",
            "DTSTAMP:20261007T120000Z\r\n",
            "DTSTART:20261103T100000Z\r\n",
            "END:VEVENT\r\n"
        ))
        .expect("missing identity");
        assert_eq!(
            bind_vevent_series(master.clone(), vec![missing_identity]),
            Err(IcalRecurrenceError::DetachedMissingRecurrenceId)
        );

        let detached_as_master = parse_vevent(concat!(
            "BEGIN:VEVENT\r\n",
            "UID:series@example.com\r\n",
            "DTSTAMP:20261007T120000Z\r\n",
            "RECURRENCE-ID:20261103T090000Z\r\n",
            "DTSTART:20261103T100000Z\r\n",
            "RRULE:FREQ=DAILY;COUNT=2\r\n",
            "END:VEVENT\r\n"
        ))
        .expect("detached-as-master");
        assert_eq!(
            bind_vevent_series(detached_as_master, Vec::new()),
            Err(IcalRecurrenceError::MasterHasRecurrenceId)
        );
    }

    #[test]
    fn vevent_series_rejects_phantom_targets_and_unmodeled_detached_semantics() {
        let master = parse_vevent(concat!(
            "BEGIN:VEVENT\r\n",
            "UID:series@example.com\r\n",
            "DTSTAMP:20261006T120000Z\r\n",
            "DTSTART:20261102T090000Z\r\n",
            "RRULE:FREQ=DAILY;COUNT=2\r\n",
            "SUMMARY:Master title\r\n",
            "END:VEVENT\r\n"
        ))
        .expect("master");

        let phantom = parse_vevent(concat!(
            "BEGIN:VEVENT\r\n",
            "UID:series@example.com\r\n",
            "DTSTAMP:20261007T120000Z\r\n",
            "RECURRENCE-ID:20261110T090000Z\r\n",
            "DTSTART:20261110T100000Z\r\n",
            "END:VEVENT\r\n"
        ))
        .expect("phantom");
        let series = bind_vevent_series(master.clone(), vec![phantom]).expect("bind phantom");
        assert!(matches!(
            series.canonical_recurrence_rule(),
            Err(IcalRecurrenceError::Domain(
                RecurrenceError::UnknownOverrideTarget(_)
            ))
        ));

        let changed_summary = parse_vevent(concat!(
            "BEGIN:VEVENT\r\n",
            "UID:series@example.com\r\n",
            "DTSTAMP:20261007T120000Z\r\n",
            "RECURRENCE-ID:20261103T090000Z\r\n",
            "DTSTART:20261103T100000Z\r\n",
            "SUMMARY:Occurrence-specific title\r\n",
            "END:VEVENT\r\n"
        ))
        .expect("changed summary");
        let series =
            bind_vevent_series(master, vec![changed_summary]).expect("bind changed summary");
        assert_eq!(
            series.canonical_recurrence_rule(),
            Err(IcalRecurrenceError::UnsupportedDetachedOverride(
                "occurrence-specific SUMMARY".to_string()
            ))
        );
    }

    #[test]
    fn vevent_series_rejects_range_this_and_future_during_identity_binding() {
        let master = parse_vevent(concat!(
            "BEGIN:VEVENT\r\n",
            "UID:series@example.com\r\n",
            "DTSTAMP:20261006T120000Z\r\n",
            "DTSTART:20261102T090000Z\r\n",
            "RRULE:FREQ=DAILY;COUNT=3\r\n",
            "END:VEVENT\r\n"
        ))
        .expect("master");
        let ranged = parse_vevent(concat!(
            "BEGIN:VEVENT\r\n",
            "UID:series@example.com\r\n",
            "DTSTAMP:20261007T120000Z\r\n",
            "RECURRENCE-ID;RANGE=THISANDFUTURE:20261103T090000Z\r\n",
            "DTSTART:20261103T100000Z\r\n",
            "END:VEVENT\r\n"
        ))
        .expect("ranged");

        assert_eq!(
            bind_vevent_series(master, vec![ranged]),
            Err(IcalRecurrenceError::UnsupportedRecurrenceRange(
                "THISANDFUTURE".to_string()
            ))
        );
    }

    #[test]
    fn vevent_time_roundtrips_all_day_with_noninclusive_end() {
        let parsed = parse_vevent_time_properties(
            "DTSTART;VALUE=DATE:20260704",
            Some("DTEND;VALUE=DATE:20260707"),
        )
        .expect("all-day VEVENT time");
        assert_eq!(
            parsed,
            TimeSpec::AllDay {
                start: NaiveDate::from_ymd_opt(2026, 7, 4).expect("start"),
                end_exclusive: Some(NaiveDate::from_ymd_opt(2026, 7, 7).expect("end")),
            }
        );
        assert_eq!(
            format_vevent_time_properties(&parsed).expect("format"),
            (
                "DTSTART;VALUE=DATE:20260704".to_string(),
                Some("DTEND;VALUE=DATE:20260707".to_string()),
            )
        );
    }

    #[test]
    fn vevent_time_roundtrips_floating_and_utc_date_times() {
        let floating =
            parse_vevent_time_properties("DTSTART:20260704T090000", Some("DTEND:20260704T103000"))
                .expect("floating VEVENT time");
        assert!(matches!(
            floating,
            TimeSpec::Floating {
                source_timezone: None,
                ..
            }
        ));
        assert_eq!(
            format_vevent_time_properties(&floating).expect("floating format"),
            (
                "DTSTART:20260704T090000".to_string(),
                Some("DTEND:20260704T103000".to_string()),
            )
        );

        let utc = parse_vevent_time_properties(
            "DTSTART:20260704T140000Z",
            Some("DTEND:20260704T153000Z"),
        )
        .expect("UTC VEVENT time");
        assert!(matches!(
            utc,
            TimeSpec::Instant {
                source_timezone: None,
                ..
            }
        ));
        assert_eq!(
            format_vevent_time_properties(&utc).expect("UTC format"),
            (
                "DTSTART:20260704T140000Z".to_string(),
                Some("DTEND:20260704T153000Z".to_string()),
            )
        );
    }

    #[test]
    fn vevent_time_roundtrips_tzid_across_dst_with_exact_duration() {
        let parsed = parse_vevent_time_properties(
            "DTSTART;TZID=America/New_York:20260308T013000",
            Some("DTEND;TZID=America/New_York:20260308T033000"),
        )
        .expect("TZID VEVENT time");

        let TimeSpec::Instant {
            start_utc,
            end_utc: Some(end_utc),
            source_timezone,
        } = &parsed
        else {
            panic!("expected zoned instant");
        };
        assert_eq!(*end_utc - *start_utc, Duration::hours(1));
        assert_eq!(source_timezone.as_deref(), Some("America/New_York"));
        assert_eq!(
            format_vevent_time_properties(&parsed).expect("TZID format"),
            (
                "DTSTART;TZID=America/New_York:20260308T013000".to_string(),
                Some("DTEND;TZID=America/New_York:20260308T033000".to_string()),
            )
        );
    }

    #[test]
    fn vevent_time_preserves_rfc_default_missing_end_shapes() {
        let all_day =
            parse_vevent_time_properties("DTSTART;VALUE=DATE:20260704", None).expect("all-day");
        assert!(matches!(
            all_day,
            TimeSpec::AllDay {
                end_exclusive: None,
                ..
            }
        ));

        let timed = parse_vevent_time_properties("DTSTART:20260704T090000Z", None).expect("timed");
        assert!(matches!(timed, TimeSpec::Instant { end_utc: None, .. }));
    }

    #[test]
    fn vevent_time_rejects_lossy_date_only_and_mismatched_end_forms() {
        let date_only = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 7, 4).expect("date"),
            end_exclusive: None,
        };
        assert!(matches!(
            format_vevent_time_properties(&date_only),
            Err(IcalRecurrenceError::UnsupportedVeventTimeKind(_))
        ));

        assert!(matches!(
            parse_vevent_time_properties(
                "DTSTART;VALUE=DATE:20260704",
                Some("DTEND:20260705T000000Z")
            ),
            Err(IcalRecurrenceError::TemporalKindMismatch { .. })
        ));

        assert!(matches!(
            parse_vevent_time_properties(
                "DTSTART;TZID=America/New_York:20260704T090000",
                Some("DTEND;TZID=America/Chicago:20260704T103000")
            ),
            Err(IcalRecurrenceError::TimezoneMismatch { .. })
        ));
    }

    #[test]
    fn vevent_time_rejects_nonpositive_end_and_unrepresentable_floating_metadata() {
        assert!(matches!(
            parse_vevent_time_properties("DTSTART:20260704T090000", Some("DTEND:20260704T090000")),
            Err(IcalRecurrenceError::InvalidPropertyValue {
                property: "DTEND",
                ..
            })
        ));

        let floating_with_metadata = TimeSpec::Floating {
            start: NaiveDate::from_ymd_opt(2026, 7, 4)
                .expect("day")
                .and_hms_opt(9, 0, 0)
                .expect("time"),
            end: None,
            source_timezone: Some("America/New_York".to_string()),
        };
        assert!(matches!(
            format_vevent_time_properties(&floating_with_metadata),
            Err(IcalRecurrenceError::UnsupportedVeventTimeKind(_))
        ));
    }

    #[test]
    fn recurrence_id_roundtrips_date_and_preserves_original_slot_shape() {
        let base = TimeSpec::DateOnly {
            start: NaiveDate::from_ymd_opt(2026, 1, 10).expect("start"),
            end_exclusive: Some(NaiveDate::from_ymd_opt(2026, 1, 12).expect("end")),
        };
        let original = parse_recurrence_id_property("RECURRENCE-ID;VALUE=DATE:20260310", &base)
            .expect("RECURRENCE-ID");

        assert_eq!(
            original,
            TimeSpec::DateOnly {
                start: NaiveDate::from_ymd_opt(2026, 3, 10).expect("original"),
                end_exclusive: Some(NaiveDate::from_ymd_opt(2026, 3, 12).expect("original end")),
            }
        );
        assert_eq!(
            format_recurrence_id_property(&original, &base).expect("format"),
            "RECURRENCE-ID;VALUE=DATE:20260310"
        );
    }

    #[test]
    fn recurrence_id_roundtrips_source_timezone_local_identity() {
        let base_start = DateTime::parse_from_rfc3339("2026-01-05T14:00:00Z")
            .expect("base")
            .with_timezone(&Utc);
        let base = TimeSpec::Instant {
            start_utc: base_start,
            end_utc: Some(base_start + Duration::hours(1)),
            source_timezone: Some("America/New_York".to_string()),
        };
        let original = parse_recurrence_id_property(
            "RECURRENCE-ID;TZID=America/New_York:20260706T090000",
            &base,
        )
        .expect("RECURRENCE-ID");

        assert_eq!(
            format_recurrence_id_property(&original, &base).expect("format"),
            "RECURRENCE-ID;TZID=America/New_York:20260706T090000"
        );
    }

    #[test]
    fn recurrence_id_rejects_range_multiple_values_and_wrong_time_form() {
        let floating = TimeSpec::Floating {
            start: NaiveDate::from_ymd_opt(2026, 1, 5)
                .expect("day")
                .and_hms_opt(9, 0, 0)
                .expect("time"),
            end: None,
            source_timezone: None,
        };
        assert_eq!(
            parse_recurrence_id_property(
                "RECURRENCE-ID;RANGE=THISANDFUTURE:20260112T090000",
                &floating
            ),
            Err(IcalRecurrenceError::UnsupportedRecurrenceRange(
                "THISANDFUTURE".to_string()
            ))
        );
        assert_eq!(
            parse_recurrence_id_property(
                "RECURRENCE-ID:20260112T090000,20260119T090000",
                &floating
            ),
            Err(IcalRecurrenceError::RecurrenceIdRequiresSingleValue)
        );

        let base_start = DateTime::parse_from_rfc3339("2026-01-05T14:00:00Z")
            .expect("base")
            .with_timezone(&Utc);
        let zoned = TimeSpec::Instant {
            start_utc: base_start,
            end_utc: None,
            source_timezone: Some("America/New_York".to_string()),
        };
        assert!(matches!(
            parse_recurrence_id_property("RECURRENCE-ID:20260112T140000Z", &zoned),
            Err(IcalRecurrenceError::TemporalKindMismatch { .. })
        ));
    }

    #[test]
    fn recurrence_id_roundtrips_utc_and_floating_forms() {
        let utc_start = DateTime::parse_from_rfc3339("2026-01-05T14:00:00Z")
            .expect("base")
            .with_timezone(&Utc);
        let utc_base = TimeSpec::Instant {
            start_utc: utc_start,
            end_utc: None,
            source_timezone: None,
        };
        let utc = parse_recurrence_id_property("RECURRENCE-ID:20260112T140000Z", &utc_base)
            .expect("UTC RECURRENCE-ID");
        assert_eq!(
            format_recurrence_id_property(&utc, &utc_base).expect("format UTC"),
            "RECURRENCE-ID:20260112T140000Z"
        );

        let floating_base = TimeSpec::Floating {
            start: NaiveDate::from_ymd_opt(2026, 1, 5)
                .expect("day")
                .and_hms_opt(9, 30, 0)
                .expect("time"),
            end: None,
            source_timezone: None,
        };
        let floating =
            parse_recurrence_id_property("RECURRENCE-ID:20260112T093000", &floating_base)
                .expect("floating RECURRENCE-ID");
        assert_eq!(
            format_recurrence_id_property(&floating, &floating_base).expect("format floating"),
            "RECURRENCE-ID:20260112T093000"
        );
    }

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
        let values = parse_exdate_property("EXDATE;VALUE=DATE:20260208", &base).expect("EXDATE");

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
        let values = parse_rdate_property("RDATE:20260702T130000Z", &base).expect("UTC RDATE");

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
        let values = parse_rdate_property("RDATE;TZID=America/New_York:20261101T013000", &base)
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
        let values = parse_rdate_property("RDATE;TZID=America/New_York:20260308T023000", &base)
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
            parse_rdate_property("RDATE;TZID=Europe/London:20260102T090000", &instant_base),
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
        let values =
            parse_exdate_property("EXDATE;TZID=\"America/New_York\":20260702T090000", &base)
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
        let values = parse_rdate_property("RDATE;VALUE=DATE:20260102,20260102,20260103", &base)
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
