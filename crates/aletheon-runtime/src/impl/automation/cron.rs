//! Cron expression parser and schedule matcher.
//!
//! Supports standard 5-field cron expressions: `minute hour dom month dow`.
//! Fields may be: `*` (any), a number, a comma-separated list, a range `a-b`,
//! or a step `*/n` or `a-b/n`.

use anyhow::{anyhow, Result};
use chrono::{DateTime, Datelike, Timelike, Utc};

/// A parsed cron schedule with one field per position.
#[derive(Debug, Clone, PartialEq)]
pub struct CronSchedule {
    pub minute: CronField,
    pub hour: CronField,
    pub day_of_month: CronField,
    pub month: CronField,
    pub day_of_week: CronField,
}

/// A single cron field value.
#[derive(Debug, Clone, PartialEq)]
pub enum CronField {
    /// `*` -- matches everything.
    Any,
    /// A single exact value.
    Exact(u32),
    /// Comma-separated list of values.
    List(Vec<u32>),
    /// Inclusive range `start-end`.
    Range { start: u32, end: u32 },
    /// Step expression: `*/n` or `start-end/n`.
    Step { every: u32, start: u32, end: u32 },
}

impl CronField {
    /// Returns `true` when `value` falls within this field.
    pub fn matches(&self, value: u32) -> bool {
        match self {
            CronField::Any => true,
            CronField::Exact(v) => *v == value,
            CronField::List(vals) => vals.contains(&value),
            CronField::Range { start, end } => value >= *start && value <= *end,
            CronField::Step { every, start, end } => {
                value >= *start && value <= *end && (value - start) % every == 0
            }
        }
    }
}

/// Stateless cron expression parser.
pub struct CronParser;

impl CronParser {
    /// Parse a 5-field cron expression into a [`CronSchedule`].
    ///
    /// # Examples
    ///
    /// ```text
    /// "*/5 * * * *"        -- every 5 minutes
    /// "0 9 * * 1-5"        -- weekdays at 09:00
    /// "0,30 8-17 * * *"    -- every 30 min during business hours
    /// ```
    pub fn parse(expr: &str) -> Result<CronSchedule> {
        let fields: Vec<&str> = expr.split_whitespace().collect();
        if fields.len() != 5 {
            return Err(anyhow!(
                "Expected 5 cron fields, got {}: '{}'",
                fields.len(),
                expr
            ));
        }

        Ok(CronSchedule {
            minute: Self::parse_field(fields[0], 0, 59)?,
            hour: Self::parse_field(fields[1], 0, 23)?,
            day_of_month: Self::parse_field(fields[2], 1, 31)?,
            month: Self::parse_field(fields[3], 1, 12)?,
            day_of_week: Self::parse_field(fields[4], 0, 7)?,
        })
    }

    /// Check whether the given UTC time matches a cron schedule.
    pub fn matches(schedule: &CronSchedule, time: &DateTime<Utc>) -> bool {
        // Cron day-of-week: 0=Sun (we accept 0 and 7 as Sunday).
        let dow = time.format("%u").to_string().parse::<u32>().unwrap_or(0);
        // Convert ISO weekday (1=Mon..7=Sun) to cron (0=Sun..6=Sat).
        let cron_dow = if dow == 7 { 0 } else { dow };

        schedule.minute.matches(time.minute())
            && schedule.hour.matches(time.hour())
            && schedule.day_of_month.matches(time.day())
            && schedule.month.matches(time.month())
            && schedule.day_of_week.matches(cron_dow)
    }

    // -- private helpers ------------------------------------------------------

    fn parse_field(raw: &str, min: u32, max: u32) -> Result<CronField> {
        if raw == "*" {
            return Ok(CronField::Any);
        }

        // Step: */n  or  a-b/n
        if let Some(step_pos) = raw.find('/') {
            let every: u32 = raw[step_pos + 1..]
                .parse()
                .map_err(|_| anyhow!("Invalid step value in '{}'", raw))?;
            if every == 0 {
                return Err(anyhow!("Step value must be > 0 in '{}'", raw));
            }
            let (start, end) = if &raw[..step_pos] == "*" {
                (min, max)
            } else {
                Self::parse_range(&raw[..step_pos], min, max)?
            };
            return Ok(CronField::Step { every, start, end });
        }

        // Range: a-b
        if raw.contains('-') {
            let (start, end) = Self::parse_range(raw, min, max)?;
            return Ok(CronField::Range { start, end });
        }

        // List: a,b,c
        if raw.contains(',') {
            let vals: Vec<u32> = raw
                .split(',')
                .map(|v| Self::parse_number(v.trim(), min, max))
                .collect::<Result<Vec<_>>>()?;
            return Ok(CronField::List(vals));
        }

        // Single number
        let n = Self::parse_number(raw, min, max)?;
        Ok(CronField::Exact(n))
    }

    fn parse_range(raw: &str, min: u32, max: u32) -> Result<(u32, u32)> {
        let parts: Vec<&str> = raw.split('-').collect();
        if parts.len() != 2 {
            return Err(anyhow!("Invalid range '{}'", raw));
        }
        let start = Self::parse_number(parts[0].trim(), min, max)?;
        let end = Self::parse_number(parts[1].trim(), min, max)?;
        if start > end {
            return Err(anyhow!("Range start {} > end {} in '{}'", start, end, raw));
        }
        Ok((start, end))
    }

    fn parse_number(raw: &str, min: u32, max: u32) -> Result<u32> {
        let n: u32 = raw
            .parse()
            .map_err(|_| anyhow!("Invalid number '{}'", raw))?;
        if n < min || n > max {
            return Err(anyhow!("Value {} out of range [{}, {}]", n, min, max));
        }
        Ok(n)
    }
}

// -- Tests --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn parse_wildcard() {
        let sched = CronParser::parse("* * * * *").unwrap();
        assert_eq!(sched.minute, CronField::Any);
        assert_eq!(sched.hour, CronField::Any);
        assert_eq!(sched.day_of_month, CronField::Any);
        assert_eq!(sched.month, CronField::Any);
        assert_eq!(sched.day_of_week, CronField::Any);
    }

    #[test]
    fn parse_exact_values() {
        let sched = CronParser::parse("30 9 15 6 3").unwrap();
        assert_eq!(sched.minute, CronField::Exact(30));
        assert_eq!(sched.hour, CronField::Exact(9));
        assert_eq!(sched.day_of_month, CronField::Exact(15));
        assert_eq!(sched.month, CronField::Exact(6));
        assert_eq!(sched.day_of_week, CronField::Exact(3));
    }

    #[test]
    fn parse_step() {
        let sched = CronParser::parse("*/15 * * * *").unwrap();
        assert_eq!(
            sched.minute,
            CronField::Step {
                every: 15,
                start: 0,
                end: 59
            }
        );
    }

    #[test]
    fn parse_range_step() {
        let sched = CronParser::parse("0-30/10 * * * *").unwrap();
        assert_eq!(
            sched.minute,
            CronField::Step {
                every: 10,
                start: 0,
                end: 30
            }
        );
    }

    #[test]
    fn parse_list() {
        let sched = CronParser::parse("0,15,30,45 * * * *").unwrap();
        assert_eq!(sched.minute, CronField::List(vec![0, 15, 30, 45]));
    }

    #[test]
    fn parse_range() {
        let sched = CronParser::parse("* 9-17 * * *").unwrap();
        assert_eq!(sched.hour, CronField::Range { start: 9, end: 17 });
    }

    #[test]
    fn parse_invalid_field_count() {
        assert!(CronParser::parse("* * *").is_err());
    }

    #[test]
    fn parse_out_of_range() {
        assert!(CronParser::parse("60 * * * *").is_err()); // minute max 59
        assert!(CronParser::parse("* 24 * * *").is_err()); // hour max 23
    }

    #[test]
    fn parse_step_zero() {
        assert!(CronParser::parse("*/0 * * * *").is_err());
    }

    #[test]
    fn matches_exact_time() {
        let sched = CronParser::parse("30 9 * * *").unwrap();
        // 2026-06-07 09:30:00 UTC -- Sunday (ISO dow=7, cron dow=0)
        let time = Utc.with_ymd_and_hms(2026, 6, 7, 9, 30, 0).unwrap();
        assert!(CronParser::matches(&sched, &time));
    }

    #[test]
    fn matches_wrong_minute() {
        let sched = CronParser::parse("30 9 * * *").unwrap();
        let time = Utc.with_ymd_and_hms(2026, 6, 7, 9, 31, 0).unwrap();
        assert!(!CronParser::matches(&sched, &time));
    }

    #[test]
    fn matches_step() {
        let sched = CronParser::parse("*/10 * * * *").unwrap();
        let t0 = Utc.with_ymd_and_hms(2026, 6, 7, 9, 0, 0).unwrap();
        let t1 = Utc.with_ymd_and_hms(2026, 6, 7, 9, 10, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2026, 6, 7, 9, 7, 0).unwrap();
        assert!(CronParser::matches(&sched, &t0));
        assert!(CronParser::matches(&sched, &t1));
        assert!(!CronParser::matches(&sched, &t2));
    }

    #[test]
    fn matches_range() {
        let sched = CronParser::parse("* 9-17 * * *").unwrap();
        let in_range = Utc.with_ymd_and_hms(2026, 6, 7, 12, 0, 0).unwrap();
        let out_range = Utc.with_ymd_and_hms(2026, 6, 7, 18, 0, 0).unwrap();
        assert!(CronParser::matches(&sched, &in_range));
        assert!(!CronParser::matches(&sched, &out_range));
    }

    #[test]
    fn matches_weekday() {
        // Mon-Fri (1-5)
        let sched = CronParser::parse("0 9 * * 1-5").unwrap();
        // 2026-06-08 is Monday (ISO dow=1, cron dow=1)
        let monday = Utc.with_ymd_and_hms(2026, 6, 8, 9, 0, 0).unwrap();
        // 2026-06-07 is Sunday (ISO dow=7, cron dow=0)
        let sunday = Utc.with_ymd_and_hms(2026, 6, 7, 9, 0, 0).unwrap();
        assert!(CronParser::matches(&sched, &monday));
        assert!(!CronParser::matches(&sched, &sunday));
    }

    #[test]
    fn field_matches_variants() {
        assert!(CronField::Any.matches(42));
        assert!(CronField::Exact(5).matches(5));
        assert!(!CronField::Exact(5).matches(6));
        assert!(CronField::List(vec![1, 3, 5]).matches(3));
        assert!(!CronField::List(vec![1, 3, 5]).matches(4));
        assert!(CronField::Range { start: 1, end: 5 }.matches(3));
        assert!(!CronField::Range { start: 1, end: 5 }.matches(6));
        assert!(CronField::Step {
            every: 5,
            start: 0,
            end: 55
        }
        .matches(25));
        assert!(!CronField::Step {
            every: 5,
            start: 0,
            end: 55
        }
        .matches(27));
    }
}
