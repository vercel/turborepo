use std::fmt::{self, Formatter};

use chrono::{DateTime, Duration, Local, SubsecRound};

#[derive(Debug)]
pub struct TurboDuration(Duration);

impl TurboDuration {
    pub fn new(start_time: &DateTime<Local>, end_time: &DateTime<Local>) -> Self {
        TurboDuration(
            end_time
                .trunc_subsecs(3)
                .signed_duration_since(start_time.trunc_subsecs(3)),
        )
    }
}

impl From<Duration> for TurboDuration {
    fn from(duration: Duration) -> Self {
        Self(duration)
    }
}

impl fmt::Display for TurboDuration {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let duration = &self.0;

        // If duration is less than a second, we print milliseconds
        if duration.num_seconds() <= 0 {
            let milliseconds = duration.num_milliseconds() - duration.num_seconds() * 1000;
            return write!(f, "{}ms", milliseconds);
        }

        if duration.num_hours() > 0 {
            write!(f, "{}h", duration.num_hours(),)?;
        }

        if duration.num_minutes() > 0 {
            let minutes = duration.num_minutes() - duration.num_hours() * 60;
            write!(f, "{}m", minutes)?;
        }

        if duration.num_seconds() > 0 {
            let seconds = duration.num_seconds() - duration.num_minutes() * 60;
            write!(f, "{}s", seconds)?;
        }

        Ok(())
    }
}
