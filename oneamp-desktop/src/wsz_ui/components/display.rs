pub struct DigitalDisplay {
    position: (u32, u32),
    pub current_time: f32,
    pub total_time: f32,
    pub show_remaining: bool,
}

/// Decoded MM:SS time as four digits + a minus flag for remaining-time
/// display. Minutes saturate at 99 — the time strip is only 4 digits wide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeDigits {
    pub minus: bool,
    pub digits: [u8; 4],
}

impl DigitalDisplay {
    pub fn new() -> Self {
        Self {
            position: (48, 26),
            current_time: 0.0,
            total_time: 0.0,
            show_remaining: false,
        }
    }

    pub fn position(&self) -> (u32, u32) {
        self.position
    }

    pub fn digit_size(&self) -> (u32, u32) {
        (9, 13)
    }

    pub fn set_time(&mut self, current: f32, total: f32) {
        self.current_time = current;
        self.total_time = total;
    }

    pub fn time_digits(&self) -> TimeDigits {
        let time = if self.show_remaining {
            -(self.total_time - self.current_time)
        } else {
            self.current_time
        };

        let minus = time < 0.0;
        let abs_secs = time.abs() as i32;
        let mins = (abs_secs / 60).clamp(0, 99) as u8;
        let secs = (abs_secs % 60) as u8;

        TimeDigits {
            minus,
            digits: [mins / 10, mins % 10, secs / 10, secs % 10],
        }
    }

    /// Plain-text rendering (used by shade mode's proportional fallback).
    pub fn format_time(&self) -> String {
        let d = self.time_digits();
        let prefix = if d.minus { "-" } else { "" };
        format!(
            "{}{}{}:{}{}",
            prefix, d.digits[0], d.digits[1], d.digits[2], d.digits[3]
        )
    }
}

impl Default for DigitalDisplay {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(c: f32, t: f32, remaining: bool) -> TimeDigits {
        let mut disp = DigitalDisplay::new();
        disp.set_time(c, t);
        disp.show_remaining = remaining;
        disp.time_digits()
    }

    #[test]
    fn zero_time() {
        let r = d(0.0, 0.0, false);
        assert!(!r.minus);
        assert_eq!(r.digits, [0, 0, 0, 0]);
    }

    #[test]
    fn three_seconds() {
        let r = d(3.0, 240.0, false);
        assert!(!r.minus);
        assert_eq!(r.digits, [0, 0, 0, 3]);
    }

    #[test]
    fn five_minutes_thirteen() {
        let r = d(313.0, 600.0, false);
        assert!(!r.minus);
        assert_eq!(r.digits, [0, 5, 1, 3]);
    }

    #[test]
    fn saturate_at_99_minutes() {
        let r = d(60.0 * 150.0, 60.0 * 200.0, false);
        assert_eq!(r.digits, [9, 9, 0, 0]);
    }

    #[test]
    fn remaining_is_negative() {
        // 4 minutes total, 1 minute elapsed → 3 minutes remaining (-3:00)
        let r = d(60.0, 240.0, true);
        assert!(r.minus);
        assert_eq!(r.digits, [0, 3, 0, 0]);
    }

    #[test]
    fn format_string_zero() {
        let mut disp = DigitalDisplay::new();
        disp.set_time(0.0, 0.0);
        assert_eq!(disp.format_time(), "00:00");
    }

    #[test]
    fn format_string_remaining() {
        let mut disp = DigitalDisplay::new();
        disp.set_time(60.0, 240.0);
        disp.show_remaining = true;
        assert_eq!(disp.format_time(), "-03:00");
    }
}
