//! The statistics page (san-1021): counts of what Sanshain holds, for
//! architects, management and maintainers, one tab per topic.
//!
//! Everything is aggregated here, on the server: a `report_viewer` holds
//! `view_statistics` and nothing else, so no answer may carry a person's name
//! or a single audit entry, not even one the page would hide. Activity is
//! counted per system and rolled up to the system's Backstage owner.

use crate::domain::models::AppError;
use chrono::{Days, NaiveDate};
use serde::Serialize;

/// The range the page shows when it names none: the last 90 days, today
/// included.
pub const DEFAULT_RANGE_DAYS: u64 = 90;

/// The days a statistics answer covers, both ends included, in UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct StatisticsRange {
    pub from: NaiveDate,
    pub to: NaiveDate,
}

impl StatisticsRange {
    /// The range of `from`/`to` (`YYYY-MM-DD`): a missing `to` is today, a
    /// missing `from` is [`DEFAULT_RANGE_DAYS`] before `to`. A range that ends
    /// before it starts, or a date that is not one, is a 400.
    pub fn parse(
        from: Option<&str>,
        to: Option<&str>,
        today: NaiveDate,
    ) -> Result<StatisticsRange, AppError> {
        let day = |name: &str, raw: &str| {
            NaiveDate::parse_from_str(raw, "%Y-%m-%d").map_err(|_| {
                AppError::BadRequest(format!("{name} must be a date (YYYY-MM-DD), got '{raw}'"))
            })
        };
        let to = match to.filter(|s| !s.is_empty()) {
            Some(raw) => day("to", raw)?,
            None => today,
        };
        let from = match from.filter(|s| !s.is_empty()) {
            Some(raw) => day("from", raw)?,
            None => to
                .checked_sub_days(Days::new(DEFAULT_RANGE_DAYS.saturating_sub(1)))
                .unwrap_or(NaiveDate::MIN),
        };
        if from > to {
            return Err(AppError::BadRequest(format!(
                "the range ends ({to}) before it starts ({from})"
            )));
        }
        Ok(StatisticsRange { from, to })
    }

    /// Every day of the range, oldest first.
    pub fn days(&self) -> Vec<NaiveDate> {
        self.from
            .iter_days()
            .take_while(|d| *d <= self.to)
            .collect()
    }
}

/// The tabs of the statistics page that have data of their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatisticsTab {}

impl StatisticsTab {
    /// The tab a route segment names, or `None` for one that does not exist.
    pub fn parse(_raw: &str) -> Option<StatisticsTab> {
        None
    }
}

/// What every statistics answer carries around its tab's data.
#[derive(Debug, Clone, Serialize)]
pub struct StatisticsAnswer<T: Serialize> {
    pub tab: &'static str,
    pub range: StatisticsRange,
    pub data: T,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn the_default_range_is_the_last_90_days_up_to_today() {
        let range = StatisticsRange::parse(None, None, d("2026-11-25")).unwrap();
        assert_eq!(range.from, d("2026-08-28"));
        assert_eq!(range.to, d("2026-11-25"));
        assert_eq!(range.days().len(), 90);
    }

    #[test]
    fn a_named_range_is_taken_as_it_is() {
        let range = StatisticsRange::parse(Some("2026-01-01"), Some("2026-01-31"), d("2026-11-25"))
            .unwrap();
        assert_eq!(range.days().len(), 31);
        let only_to = StatisticsRange::parse(None, Some("2026-01-31"), d("2026-11-25")).unwrap();
        assert_eq!(only_to.from, d("2025-11-03"));
        let empty = StatisticsRange::parse(Some(""), Some(""), d("2026-11-25")).unwrap();
        assert_eq!(empty.to, d("2026-11-25"));
    }

    #[test]
    fn a_backwards_or_broken_range_is_a_bad_request() {
        assert!(matches!(
            StatisticsRange::parse(Some("2026-02-01"), Some("2026-01-01"), d("2026-11-25")),
            Err(AppError::BadRequest(m)) if m.contains("before it starts")
        ));
        assert!(matches!(
            StatisticsRange::parse(Some("yesterday"), None, d("2026-11-25")),
            Err(AppError::BadRequest(m)) if m.contains("from must be a date")
        ));
    }

    #[test]
    fn an_unknown_tab_is_none() {
        assert_eq!(StatisticsTab::parse("nope"), None);
    }
}
