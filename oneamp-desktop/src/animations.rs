use std::time::Instant;

/// Animated value with smooth interpolation
#[derive(Debug, Clone)]
pub struct AnimatedValue {
    pub current: f32,
    pub target: f32,
    pub speed: f32, // How fast to reach target (0.0 - 1.0)
}

impl AnimatedValue {
    pub fn new(initial: f32, speed: f32) -> Self {
        Self {
            current: initial,
            target: initial,
            speed: speed.clamp(0.01, 1.0),
        }
    }

    pub fn set_target(&mut self, target: f32) {
        self.target = target;
    }

    pub fn update(&mut self, dt: f32) {
        let diff = self.target - self.current;
        self.current += diff * self.speed * (dt * 60.0); // Normalize to 60 FPS
    }

    pub fn get(&self) -> f32 {
        self.current
    }

    pub fn is_animating(&self) -> bool {
        (self.current - self.target).abs() > 0.001
    }
}

/// Easing functions for smooth animations
pub struct Easing;

impl Easing {
    /// Linear interpolation (no easing)
    pub fn linear(t: f32) -> f32 {
        t
    }

    /// Ease out cubic (fast start, slow end)
    pub fn ease_out_cubic(t: f32) -> f32 {
        1.0 - (1.0 - t).powi(3)
    }

    /// Ease in cubic (slow start, fast end)
    pub fn ease_in_cubic(t: f32) -> f32 {
        t.powi(3)
    }

    /// Ease in-out cubic (slow start and end)
    pub fn ease_in_out_cubic(t: f32) -> f32 {
        if t < 0.5 {
            4.0 * t.powi(3)
        } else {
            1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
        }
    }

    /// Ease out sine (smooth deceleration)
    pub fn ease_out_sine(t: f32) -> f32 {
        (t * std::f32::consts::FRAC_PI_2).sin()
    }

    /// Ease in-out sine (smooth acceleration and deceleration)
    pub fn ease_in_out_sine(t: f32) -> f32 {
        -(t * std::f32::consts::PI).cos() / 2.0 + 0.5
    }

    /// Elastic ease out (bouncy effect)
    pub fn ease_out_elastic(t: f32) -> f32 {
        if t == 0.0 || t == 1.0 {
            t
        } else {
            let c4 = (2.0 * std::f32::consts::PI) / 3.0;
            2.0_f32.powf(-10.0 * t) * ((t * 10.0 - 0.75) * c4).sin() + 1.0
        }
    }

    /// Bounce ease out (bouncing ball effect)
    pub fn ease_out_bounce(t: f32) -> f32 {
        const N1: f32 = 7.5625;
        const D1: f32 = 2.75;

        if t < 1.0 / D1 {
            N1 * t * t
        } else if t < 2.0 / D1 {
            let t = t - 1.5 / D1;
            N1 * t * t + 0.75
        } else if t < 2.5 / D1 {
            let t = t - 2.25 / D1;
            N1 * t * t + 0.9375
        } else {
            let t = t - 2.625 / D1;
            N1 * t * t + 0.984375
        }
    }
}

/// Animation timer for time-based effects
#[derive(Debug, Clone)]
pub struct AnimationTimer {
    start_time: Instant,
}

impl AnimationTimer {
    pub fn new() -> Self {
        Self {
            start_time: Instant::now(),
        }
    }

    /// Get elapsed time in seconds
    pub fn elapsed(&self) -> f32 {
        self.start_time.elapsed().as_secs_f32()
    }

    /// Get elapsed time modulo a period (for looping animations)
    pub fn elapsed_looped(&self, period: f32) -> f32 {
        (self.elapsed() % period) / period
    }

    /// Reset the timer
    pub fn reset(&mut self) {
        self.start_time = Instant::now();
    }
}

impl Default for AnimationTimer {
    fn default() -> Self {
        Self::new()
    }
}

/// Color animation with smooth transitions
#[derive(Debug, Clone)]
pub struct AnimatedColor {
    pub current: [f32; 3], // RGB as floats (0.0 - 1.0)
    pub target: [f32; 3],
    pub speed: f32,
}

impl AnimatedColor {
    pub fn new(initial: [u8; 3], speed: f32) -> Self {
        let initial_f = [
            initial[0] as f32 / 255.0,
            initial[1] as f32 / 255.0,
            initial[2] as f32 / 255.0,
        ];
        Self {
            current: initial_f,
            target: initial_f,
            speed: speed.clamp(0.01, 1.0),
        }
    }

    pub fn set_target(&mut self, target: [u8; 3]) {
        self.target = [
            target[0] as f32 / 255.0,
            target[1] as f32 / 255.0,
            target[2] as f32 / 255.0,
        ];
    }

    pub fn update(&mut self, dt: f32) {
        for i in 0..3 {
            let diff = self.target[i] - self.current[i];
            self.current[i] += diff * self.speed * (dt * 60.0);
        }
    }

    pub fn get_u8(&self) -> [u8; 3] {
        [
            (self.current[0] * 255.0) as u8,
            (self.current[1] * 255.0) as u8,
            (self.current[2] * 255.0) as u8,
        ]
    }

    pub fn is_animating(&self) -> bool {
        (self.current[0] - self.target[0]).abs() > 0.001
            || (self.current[1] - self.target[1]).abs() > 0.001
            || (self.current[2] - self.target[2]).abs() > 0.001
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_animated_value() {
        let mut val = AnimatedValue::new(0.0, 0.1);
        assert_eq!(val.get(), 0.0);

        val.set_target(10.0);
        val.update(1.0 / 60.0);
        assert!(val.get() > 0.0);
        assert!(val.is_animating());
    }

    #[test]
    fn test_easing_linear() {
        assert_eq!(Easing::linear(0.0), 0.0);
        assert_eq!(Easing::linear(0.5), 0.5);
        assert_eq!(Easing::linear(1.0), 1.0);
    }

    #[test]
    fn test_easing_cubic() {
        let t = 0.5;
        let result = Easing::ease_out_cubic(t);
        assert!(result > 0.0 && result < 1.0);
    }

    #[test]
    fn test_animation_timer() {
        let timer = AnimationTimer::new();
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(timer.elapsed() >= 0.1);
    }

    #[test]
    fn test_animated_color() {
        let mut color = AnimatedColor::new([255, 0, 0], 0.1);
        assert_eq!(color.get_u8(), [255, 0, 0]);

        color.set_target([0, 255, 0]);
        color.update(1.0 / 60.0);
        assert!(color.is_animating());
    }
}
