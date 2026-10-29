use std::time::{Duration, Instant};

/// Local input timing, not a model of human behavior. Entropy is supplied by
/// the caller; positions never leave the current framebuffer.
pub struct PasteTouch {
    pub point: (u16, u16),
    long_hold: Duration,
    short_hold: Duration,
    stage: u8,
    deadline: Instant,
}

impl PasteTouch {
    pub const INITIAL_DELAY: Duration = Duration::from_millis(150);
    pub const TAP_GAP: Duration = Duration::from_millis(80);
    pub fn long_hold(&self) -> Duration {
        self.long_hold
    }
    pub fn short_hold(&self) -> Duration {
        self.short_hold
    }

    pub fn new(
        anchor: (u16, u16),
        size: (u16, u16),
        entropy: [u8; 32],
        now: Instant,
    ) -> Option<Self> {
        if size.0 == 0 || size.1 == 0 || anchor.0 >= size.0 || anchor.1 >= size.1 {
            return None;
        }
        let mut normals = [0.0; 4];
        for (i, pair) in entropy.chunks_exact(16).enumerate() {
            let unit = |bytes: &[u8]| {
                // Strictly inside (0,1), so Box–Muller never takes ln(0).
                ((u64::from_le_bytes(bytes.try_into().unwrap()) >> 12) as f64 + 0.5)
                    / ((1u64 << 52) as f64)
            };
            let radius = (-2.0 * unit(&pair[..8]).ln()).sqrt();
            let angle = std::f64::consts::TAU * unit(&pair[8..]);
            normals[i * 2] = (radius * angle.cos()).clamp(-3.0, 3.0) / 3.0;
            normals[i * 2 + 1] = (radius * angle.sin()).clamp(-3.0, 3.0) / 3.0;
        }
        let coordinate = |value: u16, limit: u16, random: f64| {
            (i32::from(value) + (random * 2.0).round() as i32).clamp(0, i32::from(limit) - 1) as u16
        };
        Some(Self {
            point: (
                coordinate(anchor.0, size.0, normals[2]),
                coordinate(anchor.1, size.1, normals[3]),
            ),
            long_hold: Duration::from_millis((1000.0 + normals[0] * 100.0).round() as u64),
            // Keep the post-paste short tap in its own 100–120 ms range.
            short_hold: Duration::from_millis((110.0 + normals[1] * 10.0).round() as u64),
            stage: 0,
            deadline: now + Self::INITIAL_DELAY,
        })
    }

    pub fn poll(&mut self, now: Instant) -> Option<bool> {
        if now < self.deadline || self.finished() {
            return None;
        }
        let down = self.stage == 0 || self.stage == 2;
        let wait = match self.stage {
            0 => self.long_hold,
            1 => Self::TAP_GAP,
            2 => self.short_hold,
            _ => Duration::ZERO,
        };
        self.stage += 1;
        // A delayed render must not compress the following hold.
        self.deadline = now + wait;
        Some(down)
    }

    pub fn pressed(&self) -> bool {
        self.stage == 1 || self.stage == 3
    }
    pub fn finished(&self) -> bool {
        self.stage == 4
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_timing_coordinates_and_invalid_anchors() {
        let now = Instant::now();
        for byte in 0..=255 {
            for anchor in [(0, 0), (99, 199), (50, 100)] {
                let plan = PasteTouch::new(anchor, (100, 200), [byte; 32], now).unwrap();
                assert!((900..=1100).contains(&plan.long_hold.as_millis()));
                assert!((100..=120).contains(&plan.short_hold.as_millis()));
                assert!(plan.point.0 < 100 && plan.point.1 < 200);
                assert!(plan.point.0.abs_diff(anchor.0) <= 2);
                assert!(plan.point.1.abs_diff(anchor.1) <= 2);
            }
        }
        assert!(PasteTouch::new((0, 0), (0, 200), [0; 32], now).is_none());
        assert!(PasteTouch::new((100, 0), (100, 200), [0; 32], now).is_none());
    }

    #[test]
    fn sequence_pairs_events_and_keeps_holds_after_late_render() {
        let now = Instant::now();
        let mut plan = PasteTouch::new((5, 5), (10, 10), [42; 32], now).unwrap();
        assert_eq!(plan.poll(now), None);
        let start = now + Duration::from_secs(10);
        assert_eq!(plan.poll(start), Some(true));
        assert!(plan.pressed());
        assert_eq!(
            plan.poll(start + plan.long_hold - Duration::from_millis(1)),
            None
        );
        let up = start + plan.long_hold;
        assert_eq!(plan.poll(up), Some(false));
        assert!(!plan.pressed());
        assert_eq!(plan.poll(up + Duration::from_millis(79)), None);
        let short = up + Duration::from_millis(80);
        assert_eq!(plan.poll(short), Some(true));
        assert_eq!(plan.poll(short + plan.short_hold), Some(false));
        assert!(plan.finished());
        assert!(!plan.pressed());
        assert_eq!(plan.poll(short + Duration::from_secs(10)), None);
    }
}
