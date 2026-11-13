//! Timezone-aware 5-field cron expression parser and next-run scheduler calculations.

use agents_protocol::TriggerKind;
use chrono::{Datelike, Duration, FixedOffset, TimeZone, Timelike};

use crate::infra::error::ServerError;

/// Parsed 5-field cron schedule (`minute hour day_of_month month day_of_week`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronSpec {
    minutes: Vec<bool>,
    hours: Vec<bool>,
    days_of_month: Vec<bool>,
    months: Vec<bool>,
    days_of_week: Vec<bool>,
}

impl CronSpec {
    /// Parses a standard 5-field cron expression (`* * * * *`).
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Validation`] if syntax or field bounds are invalid.
    pub fn parse(expr: &str) -> Result<Self, ServerError> {
        let normalized = match expr.trim() {
            "@hourly" => "0 * * * *",
            "@daily" => "0 0 * * *",
            "@weekly" => "0 0 * * 0",
            other => other,
        };
        let parts: Vec<&str> = normalized.split_whitespace().collect();
        if parts.len() != 5 {
            return Err(ServerError::Validation(format!(
                "cron expression `{expr}` must have exactly 5 fields"
            )));
        }

        Ok(Self {
            minutes: parse_field(parts[0], 0, 59)?,
            hours: parse_field(parts[1], 0, 23)?,
            days_of_month: parse_field(parts[2], 1, 31)?,
            months: parse_field(parts[3], 1, 12)?,
            days_of_week: parse_dow_field(parts[4])?,
        })
    }

    /// Computes the next UTC millisecond timestamp strictly after `from_ms` in `tz`.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Validation`] if no match exists within 366 days.
    pub fn next_after_ms(&self, from_ms: u64, tz: FixedOffset) -> Result<u64, ServerError> {
        let secs = i64::try_from(from_ms / 1000).unwrap_or(i64::MAX);
        let base_utc = chrono::DateTime::from_timestamp(secs, 0)
            .ok_or_else(|| ServerError::Validation("timestamp out of range".into()))?;
        let base_local = base_utc.with_timezone(&tz);

        // Truncate to minute boundary and step forward by +1 minute.
        let mut cursor = base_local
            .with_second(0)
            .and_then(|dt| dt.with_nanosecond(0))
            .ok_or_else(|| ServerError::Internal("chrono truncation error".into()))?
            + Duration::minutes(1);

        const MAX_MINUTES: usize = 366 * 24 * 60;
        for _ in 0..MAX_MINUTES {
            let min = cursor.minute() as usize;
            let hour = cursor.hour() as usize;
            let dom = cursor.day() as usize;
            let month = cursor.month() as usize;
            let dow = cursor.weekday().num_days_from_sunday() as usize;

            if self.minutes[min]
                && self.hours[hour]
                && self.days_of_month[dom]
                && self.months[month]
                && self.days_of_week[dow]
            {
                let utc_ms = u64::try_from(cursor.timestamp_millis()).unwrap_or(0);
                return Ok(utc_ms);
            }
            cursor += Duration::minutes(1);
        }

        Err(ServerError::Validation(
            "cron expression did not match within 366 days".into(),
        ))
    }
}

/// Resolves canonical timezone identifiers and fixed numeric offsets (`+HH:MM` / `-HH:MM`).
///
/// # Errors
///
/// Returns [`ServerError::Validation`] for unrecognized timezone strings.
pub fn parse_timezone_offset(tz: &str) -> Result<FixedOffset, ServerError> {
    let trimmed = tz.trim();
    match trimmed {
        "UTC" | "Etc/UTC" | "Z" | "Europe/London" => FixedOffset::east_opt(0),
        "America/Bogota" | "America/Guayaquil" | "America/Lima" | "America/New_York" => {
            FixedOffset::west_opt(5 * 3600)
        }
        "America/Mexico_City" | "America/Chicago" => FixedOffset::west_opt(6 * 3600),
        "America/Los_Angeles" => FixedOffset::west_opt(8 * 3600),
        "America/Argentina/Buenos_Aires" | "America/Sao_Paulo" => FixedOffset::west_opt(3 * 3600),
        "Europe/Madrid" | "Europe/Paris" | "Europe/Berlin" => FixedOffset::east_opt(3600),
        "Asia/Tokyo" => FixedOffset::east_opt(9 * 3600),
        other => parse_numeric_offset(other),
    }
    .ok_or_else(|| ServerError::Validation(format!("unsupported timezone `{tz}`")))
}

fn parse_numeric_offset(raw: &str) -> Option<FixedOffset> {
    let (sign, rest) = if let Some(r) = raw.strip_prefix('+') {
        (1_i32, r)
    } else {
        let r = raw.strip_prefix('-')?;
        (-1_i32, r)
    };
    let (h_str, m_str) = rest.split_once(':')?;
    let hours: i32 = h_str.parse().ok()?;
    let mins: i32 = m_str.parse().ok()?;
    if !(0..=14).contains(&hours) || !(0..=59).contains(&mins) {
        return None;
    }
    FixedOffset::east_opt(sign * (hours * 3600 + mins * 60))
}

/// Validates trigger parameters and computes the next scheduled UTC millisecond execution.
///
/// # Errors
///
/// Returns [`ServerError::Validation`] on invalid timezone, cron syntax, or missing interval.
pub fn compute_next_run_ms(
    trigger: &TriggerKind,
    cron_expr: Option<&str>,
    interval_seconds: Option<u64>,
    timezone: &str,
    from_ms: u64,
) -> Result<Option<u64>, ServerError> {
    let offset = parse_timezone_offset(timezone)?;
    match trigger {
        TriggerKind::Cron => {
            let expr = cron_expr
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    ServerError::Validation("cron trigger requires non-empty `cronExpr`".into())
                })?;
            let spec = CronSpec::parse(expr)?;
            let next = spec.next_after_ms(from_ms, offset)?;
            Ok(Some(next))
        }
        TriggerKind::IntervalSeconds => {
            let secs = interval_seconds.ok_or_else(|| {
                ServerError::Validation("interval trigger requires `intervalSeconds`".into())
            })?;
            if secs == 0 {
                return Err(ServerError::Validation(
                    "intervalSeconds must be >= 1".into(),
                ));
            }
            Ok(Some(from_ms.saturating_add(secs.saturating_mul(1000))))
        }
        TriggerKind::Webhook | TriggerKind::Manual => Ok(None),
        TriggerKind::Unknown(other) => Err(ServerError::Validation(format!(
            "unsupported trigger kind `{other}`"
        ))),
    }
}

fn parse_dow_field(field: &str) -> Result<Vec<bool>, ServerError> {
    let mut flags = parse_field(field, 0, 7)?;
    if flags.len() > 7 && flags[7] {
        flags[0] = true;
    }
    flags.truncate(7);
    Ok(flags)
}

fn parse_field(field: &str, min: usize, max: usize) -> Result<Vec<bool>, ServerError> {
    let mut allowed = vec![false; max + 1];
    for part in field.split(',') {
        let (range_part, step) = match part.split_once('/') {
            Some((lhs, step_raw)) => {
                let s: usize = step_raw.parse().map_err(|_| {
                    ServerError::Validation(format!("invalid cron step `{step_raw}`"))
                })?;
                if s == 0 {
                    return Err(ServerError::Validation("cron step must be >= 1".into()));
                }
                (lhs, s)
            }
            None => (part, 1),
        };

        let (start, end) = if range_part == "*" {
            (min, max)
        } else if let Some((a, b)) = range_part.split_once('-') {
            let lo: usize = a
                .parse()
                .map_err(|_| ServerError::Validation(format!("invalid cron value `{a}`")))?;
            let hi: usize = b
                .parse()
                .map_err(|_| ServerError::Validation(format!("invalid cron value `{b}`")))?;
            if lo < min || hi > max || lo > hi {
                return Err(ServerError::Validation(format!(
                    "cron range `{range_part}` out of bounds ({min}..={max})"
                )));
            }
            (lo, hi)
        } else {
            let val: usize = range_part.parse().map_err(|_| {
                ServerError::Validation(format!("invalid cron token `{range_part}`"))
            })?;
            if val < min || val > max {
                return Err(ServerError::Validation(format!(
                    "cron value `{val}` out of bounds ({min}..={max})"
                )));
            }
            if step == 1 { (val, val) } else { (val, max) }
        };

        let mut cursor = start;
        while cursor <= end {
            allowed[cursor] = true;
            cursor = cursor.saturating_add(step);
        }
    }
    Ok(allowed)
}

/// Checks if timezone string represents a valid fixed timezone via chrono `FixedOffset`.
#[must_use]
pub fn format_utc_in_timezone(utc_ms: u64, tz: FixedOffset) -> Option<String> {
    let secs = i64::try_from(utc_ms / 1000).ok()?;
    let dt = chrono::DateTime::from_timestamp(secs, 0)?;
    Some(tz.from_utc_datetime(&dt.naive_utc()).to_rfc3339())
}
