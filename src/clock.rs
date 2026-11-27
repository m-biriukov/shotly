//! Local wall-clock time for file names and the log, read from the C library
//! (which follows the Mac's time zone setting).

/// A moment in local time, to the second.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub year: i32,
    /// 1–12.
    pub month: u32,
    /// 1–31.
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

impl Stamp {
    /// Now, in the Mac's time zone (UTC if that cannot be worked out).
    pub fn now() -> Self {
        // SAFETY: `time` accepts a null pointer; `localtime_r`/`gmtime_r` only
        // write into the `tm` we pass, and return null on failure.
        let tm = unsafe {
            let t = libc::time(std::ptr::null_mut());
            let mut tm: libc::tm = std::mem::zeroed();
            let ok = !libc::localtime_r(&raw const t, &raw mut tm).is_null()
                || !libc::gmtime_r(&raw const t, &raw mut tm).is_null();
            ok.then_some(tm)
        };
        tm.map_or(Self::EPOCH, |tm| Self {
            year: tm.tm_year + 1900,
            month: (tm.tm_mon + 1) as u32,
            day: tm.tm_mday as u32,
            hour: tm.tm_hour as u32,
            minute: tm.tm_min as u32,
            second: tm.tm_sec as u32,
        })
    }

    const EPOCH: Self = Self {
        year: 1970,
        month: 1,
        day: 1,
        hour: 0,
        minute: 0,
        second: 0,
    };

    /// "2026-11-10".
    pub fn date(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    /// "14.03.27": the time as macOS writes it in screenshot names.
    pub fn time_with_dots(&self) -> String {
        format!("{:02}.{:02}.{:02}", self.hour, self.minute, self.second)
    }

    /// "2026-11-10 14:03:27", for the log.
    pub fn log_line(&self) -> String {
        format!(
            "{} {:02}:{:02}:{:02}",
            self.date(),
            self.hour,
            self.minute,
            self.second
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_with_leading_zeros() {
        let s = Stamp {
            year: 2026,
            month: 3,
            day: 7,
            hour: 9,
            minute: 5,
            second: 0,
        };
        assert_eq!(s.date(), "2026-03-07");
        assert_eq!(s.time_with_dots(), "09.05.00");
        assert_eq!(s.log_line(), "2026-03-07 09:05:00");
    }

    #[test]
    fn now_is_a_real_date() {
        let s = Stamp::now();
        assert!(s.year >= 2024, "{s:?}");
        assert!((1..=12).contains(&s.month) && (1..=31).contains(&s.day));
        // Up to 60: a leap second.
        assert!(s.hour < 24 && s.minute < 60 && s.second <= 60);
    }
}
