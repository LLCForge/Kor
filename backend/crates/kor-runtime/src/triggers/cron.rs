//! Five-field cron schedules (`min hour day month weekday`, local time) for `cron` triggers.
//! `*`, lists, ranges and steps; weekday 0 or 7 is Sunday. Day-of-month and weekday combine like
//! Vixie cron: when both are restricted, either may match.

use chrono::{Datelike as _, Duration, Local, NaiveDateTime, TimeZone as _, Timelike as _};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schedule {
    minutes: u64,
    hours: u64,
    days: u64,
    months: u64,
    weekdays: u64,
    any_day: bool,
    any_weekday: bool,
}

fn field(text: &str, lo: u32, hi: u32) -> Result<u64, String> {
    let mut bits = 0u64;
    for part in text.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((r, s)) => (
                r,
                s.parse::<u32>().map_err(|_| format!("bad step `{part}`"))?,
            ),
            None => (part, 1),
        };
        if step == 0 {
            return Err(format!("bad step `{part}`"));
        }
        let (a, b) = if range == "*" {
            (lo, hi)
        } else if let Some((a, b)) = range.split_once('-') {
            let n = |s: &str| s.parse::<u32>().map_err(|_| format!("bad value `{part}`"));
            (n(a)?, n(b)?)
        } else {
            let v = range
                .parse::<u32>()
                .map_err(|_| format!("bad value `{part}`"))?;
            // `5/15` means from 5 to the end.
            (v, if part.contains('/') { hi } else { v })
        };
        if a < lo || b > hi || a > b {
            return Err(format!("`{part}` is outside {lo}-{hi}"));
        }
        let mut v = a;
        while v <= b {
            bits |= 1 << v;
            v += step;
        }
    }
    Ok(bits)
}

impl Schedule {
    /// Parse `0 3 * * *`.
    ///
    /// # Errors
    /// A message naming the bad field.
    pub fn parse(text: &str) -> Result<Self, String> {
        let fields: Vec<&str> = text.split_whitespace().collect();
        let [minute, hour, day, month, weekday] = fields.as_slice() else {
            return Err("a schedule has five fields: minute hour day month weekday".into());
        };
        let mut weekdays = field(weekday, 0, 7)?;
        if weekdays & (1 << 7) != 0 {
            weekdays |= 1;
        }
        Ok(Self {
            minutes: field(minute, 0, 59)?,
            hours: field(hour, 0, 23)?,
            days: field(day, 1, 31)?,
            months: field(month, 1, 12)?,
            weekdays,
            any_day: *day == "*",
            any_weekday: *weekday == "*",
        })
    }

    fn day_matches(&self, t: &NaiveDateTime) -> bool {
        let dom = self.days & (1 << t.day()) != 0;
        let dow = self.weekdays & (1 << t.weekday().num_days_from_sunday()) != 0;
        match (self.any_day, self.any_weekday) {
            (true, true) => true,
            (false, true) => dom,
            (true, false) => dow,
            (false, false) => dom || dow,
        }
    }

    /// The first matching minute strictly after `after` (naive local time); `None` if none in
    /// the next five years (`0 0 31 2 *`).
    pub fn next_after(&self, after: NaiveDateTime) -> Option<NaiveDateTime> {
        let mut t = after.with_second(0)?.with_nanosecond(0)? + Duration::minutes(1);
        let end = after + Duration::days(5 * 366);
        while t < end {
            if self.months & (1 << t.month()) == 0 {
                t = (t.date().with_day(1)? + chrono::Months::new(1)).and_hms_opt(0, 0, 0)?;
                continue;
            }
            if !self.day_matches(&t) {
                t = t.date().succ_opt()?.and_hms_opt(0, 0, 0)?;
                continue;
            }
            if self.hours & (1 << t.hour()) == 0 {
                t = t.with_minute(0)? + Duration::hours(1);
                continue;
            }
            if self.minutes & (1 << t.minute()) == 0 {
                t += Duration::minutes(1);
                continue;
            }
            return Some(t);
        }
        None
    }

    /// Next run in Unix milliseconds after `now_ms`, in the machine's local time.
    pub fn next_ms(&self, now_ms: u64) -> Option<u64> {
        let now = Local
            .timestamp_millis_opt(i64::try_from(now_ms).ok()?)
            .single()?
            .naive_local();
        let next = self.next_after(now)?;
        // A local time skipped by a DST change has no instant; take the hour after.
        let at = Local.from_local_datetime(&next).earliest().or_else(|| {
            Local
                .from_local_datetime(&(next + Duration::hours(1)))
                .earliest()
        })?;
        u64::try_from(at.timestamp_millis()).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, mo, d)
            .unwrap()
            .and_hms_opt(h, mi, 0)
            .unwrap()
    }

    #[test]
    fn next_runs() {
        let s = Schedule::parse("0 3 * * *").unwrap();
        assert_eq!(
            s.next_after(at(2026, 11, 10, 2, 59)),
            Some(at(2026, 11, 10, 3, 0))
        );
        assert_eq!(
            s.next_after(at(2026, 11, 10, 3, 0)),
            Some(at(2026, 11, 11, 3, 0))
        );
        let s = Schedule::parse("*/15 9-17 * * 1-5").unwrap();
        // 2026-11-14 is a Saturday.
        assert_eq!(
            s.next_after(at(2026, 11, 13, 17, 50)),
            Some(at(2026, 11, 16, 9, 0))
        );
        assert_eq!(
            s.next_after(at(2026, 11, 16, 9, 1)),
            Some(at(2026, 11, 16, 9, 15))
        );
        let s = Schedule::parse("30 0 1 1 7").unwrap();
        // Day 1 or a Sunday (Vixie rule): 2027-01-03 is a Sunday, but Jan 1 comes first.
        assert_eq!(
            s.next_after(at(2026, 12, 31, 0, 0)),
            Some(at(2027, 1, 1, 0, 30))
        );
        assert_eq!(
            Schedule::parse("0 0 31 2 *")
                .unwrap()
                .next_after(at(2026, 1, 1, 0, 0)),
            None
        );
    }

    #[test]
    fn bad_schedules_are_named() {
        for bad in [
            "",
            "* * * *",
            "60 * * * *",
            "* 24 * * *",
            "*/0 * * * *",
            "a * * * *",
            "5-1 * * * *",
        ] {
            assert!(Schedule::parse(bad).is_err(), "{bad}");
        }
    }
}
