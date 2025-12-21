use chrono::{NaiveDate, NaiveDateTime};

use crate::error::CoreError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeRange {
    pub start: Option<NaiveDate>,
    pub end: Option<NaiveDate>,
}

impl TimeRange {
    pub fn parse(input: &str) -> Result<Self, CoreError> {
        let trimmed = input.trim();
        let Some((left, right)) = trimmed.split_once('~') else {
            return Err(CoreError::InvalidTimeRange(trimmed.to_string()));
        };
        let start = if left.trim().is_empty() {
            None
        } else {
            Some(parse_date(left.trim())?)
        };
        let end = if right.trim().is_empty() {
            None
        } else {
            Some(parse_date(right.trim())?)
        };
        if start.is_none() && end.is_none() {
            return Err(CoreError::InvalidTimeRange(trimmed.to_string()));
        }
        if let (Some(start), Some(end)) = (start, end) {
            if start > end {
                return Err(CoreError::InvalidTimeRange(trimmed.to_string()));
            }
            return Ok(TimeRange {
                start: Some(start),
                end: Some(end),
            });
        }
        Ok(TimeRange { start, end })
    }

    pub fn to_timestamp_bounds(&self) -> (Option<i64>, Option<i64>) {
        let start_ts = self
            .start
            .map(|date| NaiveDateTime::new(date, chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap()))
            .map(|dt| dt.and_utc().timestamp());
        let end_ts = self.end.map(|date| {
            let end_time = chrono::NaiveTime::from_hms_opt(23, 59, 59).unwrap();
            NaiveDateTime::new(date, end_time).and_utc().timestamp()
        });
        (start_ts, end_ts)
    }
}

fn parse_date(input: &str) -> Result<NaiveDate, CoreError> {
    NaiveDate::parse_from_str(input, "%Y-%m-%d")
        .map_err(|_| CoreError::InvalidTimeRange(input.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_start_only_range() {
        let range = TimeRange::parse("2020-01-01~").unwrap();
        assert_eq!(range.start, Some(NaiveDate::from_ymd_opt(2020, 1, 1).unwrap()));
        assert_eq!(range.end, None);
    }

    #[test]
    fn parse_end_only_range() {
        let range = TimeRange::parse("~2020-01-01").unwrap();
        assert_eq!(range.start, None);
        assert_eq!(range.end, Some(NaiveDate::from_ymd_opt(2020, 1, 1).unwrap()));
    }

    #[test]
    fn parse_full_range() {
        let range = TimeRange::parse("2018-01-01~2020-01-01").unwrap();
        assert_eq!(range.start, Some(NaiveDate::from_ymd_opt(2018, 1, 1).unwrap()));
        assert_eq!(range.end, Some(NaiveDate::from_ymd_opt(2020, 1, 1).unwrap()));
    }

    #[test]
    fn reject_empty_range() {
        assert!(TimeRange::parse("~").is_err());
    }

    #[test]
    fn reject_inverted_range() {
        assert!(TimeRange::parse("2021-01-01~2020-01-01").is_err());
    }
}
