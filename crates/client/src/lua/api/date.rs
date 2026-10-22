//! `date` and `time`, the two `os` functions the 1.12 interface reaches as
//! flat globals, and the calendar arithmetic behind `date`.

/// `date` and `time`, the two `os` functions 1.12 exposes as flat
/// globals. 1.12's state has them and no `os` table, and addons read the
/// clock through them (pfUI's first line after load is `date("%d")`).
/// `date(format, t)` supports the `strftime` subset below, or returns a
/// table for `"*t"`; the zone is UTC, since the interpreter has no zone
/// table, and a leading `!` (5.1's explicit UTC) gives the same result.
/// `time()` is seconds since the epoch; `time(table)` does not convert the
/// table and returns the current time.
pub(in crate::lua) fn register(lua: &mlua::Lua) -> mlua::Result<()> {
    let globals = lua.globals();
    let date = lua.create_function(|lua, (format, at): (Option<String>, Option<f64>)| {
        let seconds = at.map_or_else(now_seconds, |t| t as i64);
        let civil = Civil::from_seconds(seconds);
        let format = format.unwrap_or_else(|| "%c".to_string());
        let format = format.trim_start_matches('!');
        if format.starts_with("*t") {
            let table = lua.create_table()?;
            table.set("year", civil.year)?;
            table.set("month", civil.month)?;
            table.set("day", civil.day)?;
            table.set("hour", civil.hour)?;
            table.set("min", civil.minute)?;
            table.set("sec", civil.second)?;
            table.set("wday", civil.weekday + 1)?;
            table.set("yday", civil.yearday + 1)?;
            table.set("isdst", false)?;
            return Ok(mlua::Value::Table(table));
        }
        Ok(mlua::Value::String(lua.create_string(civil.format(format))?))
    })?;
    globals.set("date", date)?;
    let time = lua.create_function(|_, _: mlua::MultiValue| Ok(now_seconds()))?;
    globals.set("time", time)?;
    Ok(())
}

/// Seconds since the Unix epoch, for `time()` and a bare `date()`.
fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// A calendar date and time of day, UTC, for the `date` function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::lua) struct Civil {
    pub year: i64,
    pub month: i64,
    pub day: i64,
    pub hour: i64,
    pub minute: i64,
    pub second: i64,
    /// 0 Sunday .. 6 Saturday.
    pub weekday: i64,
    /// 0-based day of the year.
    pub yearday: i64,
}

impl Civil {
    /// The proleptic Gregorian date of a Unix time: the days-from-civil
    /// inverse, as in Howard Hinnant's algorithms.
    pub(in crate::lua) fn from_seconds(seconds: i64) -> Civil {
        let days = seconds.div_euclid(86_400);
        let of_day = seconds.rem_euclid(86_400);
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = if month <= 2 { y + 1 } else { y };
        let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
        let before: [i64; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
        let yearday = before[(month - 1) as usize] + day - 1 + i64::from(leap && month > 2);
        Civil {
            year,
            month,
            day,
            hour: of_day / 3_600,
            minute: of_day % 3_600 / 60,
            second: of_day % 60,
            weekday: (days + 4).rem_euclid(7),
            yearday,
        }
    }

    /// The `strftime` codes an addon uses: the numeric fields, the English
    /// names, and the three composite forms. An unknown code is kept as
    /// written.
    pub(in crate::lua) fn format(&self, format: &str) -> String {
        const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
        const MONTHS: [&str; 12] = [
            "January", "February", "March", "April", "May", "June", "July", "August", "September",
            "October", "November", "December",
        ];
        let mut out = String::new();
        let mut chars = format.chars();
        while let Some(c) = chars.next() {
            if c != '%' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('Y') => out.push_str(&self.year.to_string()),
                Some('y') => out.push_str(&format!("{:02}", self.year.rem_euclid(100))),
                Some('m') => out.push_str(&format!("{:02}", self.month)),
                Some('d') => out.push_str(&format!("{:02}", self.day)),
                Some('H') => out.push_str(&format!("{:02}", self.hour)),
                Some('I') => out.push_str(&format!("{:02}", if self.hour % 12 == 0 { 12 } else { self.hour % 12 })),
                Some('M') => out.push_str(&format!("{:02}", self.minute)),
                Some('S') => out.push_str(&format!("{:02}", self.second)),
                Some('p') => out.push_str(if self.hour < 12 { "AM" } else { "PM" }),
                Some('j') => out.push_str(&format!("{:03}", self.yearday + 1)),
                Some('w') => out.push_str(&self.weekday.to_string()),
                Some('A') => out.push_str(DAYS[self.weekday as usize]),
                Some('a') => out.push_str(&DAYS[self.weekday as usize][..3]),
                Some('B') => out.push_str(MONTHS[(self.month - 1) as usize]),
                Some('b') => out.push_str(&MONTHS[(self.month - 1) as usize][..3]),
                Some('x') => out.push_str(&self.format("%m/%d/%y")),
                Some('X') => out.push_str(&self.format("%H:%M:%S")),
                Some('c') => out.push_str(&self.format("%a %b %d %H:%M:%S %Y")),
                Some('%') => out.push('%'),
                Some(other) => {
                    out.push('%');
                    out.push(other);
                }
                None => out.push('%'),
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::host::LuaHost;

    /// `date` is 5.0's flat `os.date`, in UTC: a known epoch second formats as
    /// the correct date, in every code an addon uses.
    #[test]
    fn date_reads_the_calendar_in_utc() {
        // 2026-09-07 17:44:05 UTC, a Monday, the 250th day of a common year.
        let civil = Civil::from_seconds(1_788_803_045);
        assert_eq!((civil.year, civil.month, civil.day), (2026, 9, 7));
        assert_eq!((civil.hour, civil.minute, civil.second), (17, 44, 5));
        assert_eq!((civil.weekday, civil.yearday), (1, 249));
        assert_eq!(civil.format("%Y-%m-%d %H:%M:%S %a %b %j %p %I %%"), "2026-09-07 17:44:05 Mon Sep 250 PM 05 %");
        assert_eq!(civil.format("%c"), "Mon Sep 07 17:44:05 2026");
        // The epoch itself, and a leap year's last day.
        let epoch = Civil::from_seconds(0);
        assert_eq!((epoch.year, epoch.month, epoch.day, epoch.weekday), (1970, 1, 1, 4));
        let leap = Civil::from_seconds(1_072_915_199);
        assert_eq!((leap.year, leap.month, leap.day, leap.yearday), (2003, 12, 31, 364));
        let leap = Civil::from_seconds(1_104_537_599);
        assert_eq!((leap.year, leap.month, leap.day, leap.yearday), (2004, 12, 31, 365));

        let host = LuaHost::new().expect("the interpreter starts");
        assert_eq!(host.eval_for_test("type(date)"), "function");
        assert_eq!(host.eval_for_test("date(\"%Y\", 0)"), "1970");
        assert_eq!(host.eval_for_test("date(\"*t\", 86400).day"), "2");
        assert_eq!(host.eval_for_test("tostring(time() > 1700000000)"), "true");
    }
}
