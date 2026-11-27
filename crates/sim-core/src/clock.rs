//! The one game clock. Zoom never changes it; time-lapse is a multiplier on it.
//! At 1x, one real second is one game minute, so a game day is 24 real minutes.

use serde::{Deserialize, Serialize};

pub const GAME_SECONDS_PER_REAL_SECOND: f64 = 60.0;
pub const SECONDS_PER_DAY: f64 = 86_400.0;
/// Budgets settle monthly; months are 30 game days.
pub const DAYS_PER_MONTH: u64 = 30;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Clock {
    /// Game seconds since the scenario started (day 0, 00:00).
    pub game_seconds: f64,
    /// Time-lapse multiplier (1, 2, 10, 60 in the UI; any value > 0 works).
    pub timelapse: f64,
    pub paused: bool,
}

impl Default for Clock {
    fn default() -> Self {
        // Start at 05:00 on day 0 so the first thing the player sees is the AM peak.
        Clock {
            game_seconds: 5.0 * 3600.0,
            timelapse: 1.0,
            paused: false,
        }
    }
}

impl Clock {
    pub fn day(&self) -> u64 {
        (self.game_seconds / SECONDS_PER_DAY) as u64
    }

    pub fn month(&self) -> u64 {
        self.day() / DAYS_PER_MONTH
    }

    /// Hour of day as a decimal (0..24).
    pub fn hour_of_day(&self) -> f64 {
        (self.game_seconds % SECONDS_PER_DAY) / 3600.0
    }

    pub fn set_timelapse(&mut self, x: f64) {
        if x > 0.0 && x.is_finite() {
            self.timelapse = x;
        }
    }

    /// Advance by `real_dt` real seconds. Returns how many day boundaries were crossed.
    pub fn advance(&mut self, real_dt: f64) -> u64 {
        if self.paused || real_dt.is_nan() || real_dt <= 0.0 {
            return 0;
        }
        let before = self.day();
        self.game_seconds += real_dt * GAME_SECONDS_PER_REAL_SECOND * self.timelapse;
        self.day() - before
    }

    /// Jump to 00:00 on the first day of the next month. Returns days crossed.
    pub fn skip_to_next_month(&mut self) -> u64 {
        let before = self.day();
        let next = (self.month() + 1) * DAYS_PER_MONTH;
        self.game_seconds = next as f64 * SECONDS_PER_DAY;
        self.day() - before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_real_second_is_one_game_minute() {
        let mut c = Clock {
            game_seconds: 0.0,
            ..Default::default()
        };
        c.advance(1.0);
        assert_eq!(c.game_seconds, 60.0);
        c.set_timelapse(60.0);
        // A full day at 60x takes 24 real seconds.
        let days = c.advance(24.0);
        assert_eq!(days, 1);
    }

    #[test]
    fn pause_and_month_skip() {
        let mut c = Clock {
            paused: true,
            ..Default::default()
        };
        assert_eq!(c.advance(100.0), 0);
        c.paused = false;
        assert_eq!(c.skip_to_next_month(), 30);
        assert_eq!(c.month(), 1);
        assert_eq!(c.hour_of_day(), 0.0);
    }
}
