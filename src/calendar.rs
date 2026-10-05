use chrono::{Datelike, Days, Duration, NaiveDate};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CalendarLayout {
    #[default]
    Grid,
    Agenda,
    Stream,
    Table,
}

impl CalendarLayout {
    pub const ALL: [Self; 4] = [Self::Grid, Self::Agenda, Self::Stream, Self::Table];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Grid => "Grid",
            Self::Agenda => "Agenda",
            Self::Stream => "Stream",
            Self::Table => "Table",
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Grid => "grid",
            Self::Agenda => "agenda",
            Self::Stream => "stream",
            Self::Table => "table",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw {
            "agenda" => Self::Agenda,
            "stream" => Self::Stream,
            "table" => Self::Table,
            _ => Self::Grid,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CalendarView {
    Year,
    Quarter,
    Month,
    Week,
    Day,
}

impl CalendarView {
    pub const ALL: [Self; 5] = [
        Self::Year,
        Self::Quarter,
        Self::Month,
        Self::Week,
        Self::Day,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Year => "Year",
            Self::Quarter => "Quarter",
            Self::Month => "Month",
            Self::Week => "Week",
            Self::Day => "Day",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateWindow {
    pub start: NaiveDate,
    pub end_exclusive: NaiveDate,
}

pub fn shift_focus(view: CalendarView, focus: NaiveDate, amount: i32) -> NaiveDate {
    match view {
        CalendarView::Year => NaiveDate::from_ymd_opt(focus.year() + amount, 1, 1).unwrap_or(focus),
        CalendarView::Quarter => shift_month_focus(focus, amount * 3),
        CalendarView::Month => shift_month_focus(focus, amount),
        CalendarView::Week => focus + Duration::days(i64::from(amount) * 7),
        CalendarView::Day => focus + Duration::days(i64::from(amount)),
    }
}

fn shift_month_focus(focus: NaiveDate, amount: i32) -> NaiveDate {
    let mut year = focus.year();
    let mut month = focus.month() as i32 + amount;

    while month < 1 {
        month += 12;
        year -= 1;
    }
    while month > 12 {
        month -= 12;
        year += 1;
    }

    NaiveDate::from_ymd_opt(year, month as u32, 1).unwrap_or(focus)
}

pub fn month_grid_start(focus: NaiveDate, monday_start: bool) -> NaiveDate {
    let first = NaiveDate::from_ymd_opt(focus.year(), focus.month(), 1).unwrap_or(focus);
    let weekday = if monday_start {
        first.weekday().num_days_from_monday()
    } else {
        first.weekday().num_days_from_sunday()
    };
    first - Duration::days(i64::from(weekday))
}

pub fn month_days(start: NaiveDate) -> Vec<NaiveDate> {
    (0..42)
        .filter_map(|offset| start.checked_add_days(Days::new(offset)))
        .collect()
}

pub fn quarter_months(focus: NaiveDate) -> Vec<NaiveDate> {
    let quarter_start = ((focus.month() - 1) / 3) * 3 + 1;
    (0..3)
        .filter_map(|offset| NaiveDate::from_ymd_opt(focus.year(), quarter_start + offset, 1))
        .collect()
}

pub fn week_days(focus: NaiveDate, monday_start: bool) -> Vec<NaiveDate> {
    let weekday = if monday_start {
        focus.weekday().num_days_from_monday()
    } else {
        focus.weekday().num_days_from_sunday()
    };
    let start = focus - Duration::days(i64::from(weekday));
    (0..7)
        .filter_map(|offset| start.checked_add_days(Days::new(offset)))
        .collect()
}

pub fn year_months(focus: NaiveDate) -> Vec<NaiveDate> {
    (1..=12)
        .filter_map(|month| NaiveDate::from_ymd_opt(focus.year(), month, 1))
        .collect()
}

pub fn window_for_view(view: CalendarView, focus: NaiveDate, monday_start: bool) -> DateWindow {
    match view {
        CalendarView::Year => {
            let start = NaiveDate::from_ymd_opt(focus.year(), 1, 1).unwrap_or(focus);
            let end_exclusive = NaiveDate::from_ymd_opt(focus.year() + 1, 1, 1)
                .unwrap_or_else(|| start + Duration::days(366));
            DateWindow {
                start,
                end_exclusive,
            }
        }
        CalendarView::Quarter => {
            let start_month = ((focus.month() - 1) / 3) * 3 + 1;
            let start = NaiveDate::from_ymd_opt(focus.year(), start_month, 1).unwrap_or(focus);
            let end_exclusive = shift_month_focus(start, 3);
            DateWindow {
                start,
                end_exclusive,
            }
        }
        CalendarView::Month => {
            let start = NaiveDate::from_ymd_opt(focus.year(), focus.month(), 1).unwrap_or(focus);
            let end_exclusive = shift_month_focus(start, 1);
            DateWindow {
                start,
                end_exclusive,
            }
        }
        CalendarView::Week => {
            let days = week_days(focus, monday_start);
            let start = days.first().copied().unwrap_or(focus);
            DateWindow {
                start,
                end_exclusive: start + Duration::days(7),
            }
        }
        CalendarView::Day => DateWindow {
            start: focus,
            end_exclusive: focus + Duration::days(1),
        },
    }
}

pub fn calendar_title(view: CalendarView, focus: NaiveDate, monday_start: bool) -> String {
    match view {
        CalendarView::Year => focus.format("%Y").to_string(),
        CalendarView::Quarter => {
            let quarter = ((focus.month() - 1) / 3) + 1;
            format!("Q{quarter} {}", focus.year())
        }
        CalendarView::Month => focus.format("%B %Y").to_string(),
        CalendarView::Week => {
            let window = window_for_view(CalendarView::Week, focus, monday_start);
            let end = window.end_exclusive - Duration::days(1);
            format!(
                "{} - {}",
                window.start.format("%b %e"),
                end.format("%b %e, %Y")
            )
        }
        CalendarView::Day => focus.format("%A, %B %e, %Y").to_string(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn calendar_layout_roundtrips_persisted_names() {
        for layout in CalendarLayout::ALL {
            assert_eq!(CalendarLayout::parse(layout.as_str()), layout);
        }
    }

    #[test]
    fn shift_focus_week_moves_by_seven_days() {
        let focus = NaiveDate::from_ymd_opt(2026, 5, 22).expect("valid test date");
        assert_eq!(
            shift_focus(CalendarView::Week, focus, 1),
            focus + Duration::days(7)
        );
    }

    #[test]
    fn month_window_is_half_open() {
        let focus = NaiveDate::from_ymd_opt(2026, 10, 15).expect("valid test date");
        let window = window_for_view(CalendarView::Month, focus, false);
        assert_eq!(
            window.start,
            NaiveDate::from_ymd_opt(2026, 10, 1).expect("valid date")
        );
        assert_eq!(
            window.end_exclusive,
            NaiveDate::from_ymd_opt(2026, 11, 1).expect("valid date")
        );
    }

    #[test]
    fn week_window_respects_week_start() {
        let focus = NaiveDate::from_ymd_opt(2026, 10, 4).expect("valid date");
        let sunday = window_for_view(CalendarView::Week, focus, false);
        let monday = window_for_view(CalendarView::Week, focus, true);

        assert_eq!(sunday.start, focus);
        assert_eq!(
            monday.start,
            NaiveDate::from_ymd_opt(2026, 9, 28).expect("valid date")
        );
    }
}
