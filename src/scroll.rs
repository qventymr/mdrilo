use windows::Win32::UI::WindowsAndMessaging::WHEEL_DELTA;

pub const AUTOSCROLL_DEAD_ZONE: f32 = 10.0;

const ZOOM_LEVELS: [f32; 13] = [
    0.50, 0.67, 0.75, 0.80, 0.90, 1.0, 1.10, 1.25, 1.50, 1.75, 2.0, 2.50, 3.0,
];
const DEFAULT_ZOOM_LEVEL: usize = 5;

pub struct Zoom {
    level: usize,
    wheel_remainder: i32,
}

impl Default for Zoom {
    fn default() -> Self {
        Self {
            level: DEFAULT_ZOOM_LEVEL,
            wheel_remainder: 0,
        }
    }
}

impl Zoom {
    pub fn scale(&self) -> f32 {
        ZOOM_LEVELS[self.level]
    }

    pub fn wheel(&mut self, delta: i16) -> bool {
        let ticks = self.wheel_remainder + i32::from(delta);
        self.wheel_remainder = ticks % WHEEL_DELTA as i32;
        let steps = ticks / WHEEL_DELTA as i32;
        let next = (self.level as i32 + steps).clamp(0, ZOOM_LEVELS.len() as i32 - 1) as usize;
        let changed = next != self.level;
        self.level = next;
        changed
    }
}

pub fn autoscroll_speed(distance: f32) -> f32 {
    let offset = (distance.abs() - AUTOSCROLL_DEAD_ZONE).max(0.0);
    let speed = (offset * (3.8 + offset * 0.038)).min(2600.0);
    speed.copysign(distance)
}

pub struct WheelMotion {
    pub target: f32,
    velocity: f32,
}

impl WheelMotion {
    pub fn retarget(previous: Option<&Self>, position: f32, distance: f32, limit: f32) -> Self {
        let (base, velocity) = previous
            .filter(|motion| (motion.target - position) * distance > 0.0)
            .map_or((position, 0.0), |motion| (motion.target, motion.velocity));
        Self {
            target: (base + distance).clamp(0.0, limit),
            velocity,
        }
    }

    pub fn advance(&mut self, position: f32, seconds: f32) -> f32 {
        // A critically damped spring preserves speed when another notch changes the target.
        const RATE: f32 = 31.0;
        let offset = position - self.target;
        let response = self.velocity + RATE * offset;
        let decay = (-RATE * seconds).exp();
        let remaining = (offset + response * seconds) * decay;
        self.velocity = (self.velocity - RATE * response * seconds) * decay;
        if remaining.abs() <= 0.25 || remaining * offset <= 0.0 {
            self.target
        } else {
            self.target + remaining
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accumulates_partial_wheel_ticks_for_zoom() {
        let mut zoom = Zoom::default();
        assert_eq!(zoom.scale(), 1.0);
        assert!(!zoom.wheel(60));
        assert!(zoom.wheel(60));
        assert_eq!(zoom.scale(), 1.10);
        assert!(zoom.wheel(-120));
        assert_eq!(zoom.scale(), 1.0);
    }

    #[test]
    fn bounds_zoom_in_both_directions() {
        let mut zoom = Zoom::default();
        assert!(zoom.wheel(1200));
        assert_eq!(zoom.scale(), 3.0);
        assert!(!zoom.wheel(120));
        assert!(zoom.wheel(-2400));
        assert_eq!(zoom.scale(), 0.50);
        assert!(!zoom.wheel(-120));
    }

    #[test]
    fn consecutive_notches_preserve_motion_and_reach_the_combined_target() {
        let mut motion = WheelMotion::retarget(None, 0.0, 87.0, 800.0);
        let mut position = motion.advance(0.0, 0.016);
        assert!(position > 0.0 && position < 87.0);
        let speed = motion.velocity;
        let mut motion = WheelMotion::retarget(Some(&motion), position, 87.0, 800.0);
        assert_eq!(motion.velocity, speed);
        assert_eq!(motion.target, 174.0);
        for _ in 0..30 {
            let next = motion.advance(position, 0.016);
            assert!(next >= position && next <= motion.target);
            position = next;
        }
        assert_eq!(position, 174.0);
    }

    #[test]
    fn reversing_the_wheel_starts_from_the_displayed_position() {
        let mut motion = WheelMotion::retarget(None, 300.0, 87.0, 800.0);
        let position = motion.advance(300.0, 0.048);
        let mut reverse = WheelMotion::retarget(Some(&motion), position, -87.0, 800.0);
        assert_eq!(reverse.target, position - 87.0);
        assert!(reverse.advance(position, 0.016) < position);
    }

    #[test]
    fn autoscroll_is_still_near_the_anchor_and_faster_farther_away() {
        assert_eq!(autoscroll_speed(5.0), 0.0);
        assert_eq!(autoscroll_speed(-10.0), 0.0);
        assert!(autoscroll_speed(40.0) > autoscroll_speed(20.0));
        assert_eq!(autoscroll_speed(-40.0), -autoscroll_speed(40.0));
        assert_eq!(autoscroll_speed(2000.0), 2600.0);
    }
}
