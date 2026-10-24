use std::collections::VecDeque;
use std::time::{Duration, Instant};

use vnc::ClientMouseEvent;

const MIN_CLICK_HOLD: Duration = Duration::from_millis(100);

/// Preserve pointer ordering while holding a stationary left click for at least
/// 100 ms. Keys, frame decoding and connection shutdown never wait on this timer.
#[derive(Default)]
pub(crate) struct PointerTiming {
    pending: VecDeque<ClientMouseEvent>,
    sent: Option<ClientMouseEvent>,
    down_at: Option<Instant>,
    dragged: bool,
}

impl PointerTiming {
    pub fn push(&mut self, event: ClientMouseEvent) {
        self.pending.push_back(event);
    }

    pub fn pop_ready(&mut self, now: Instant) -> Option<ClientMouseEvent> {
        let next = self.pending.front()?;
        let was_down = self.sent.as_ref().is_some_and(|e| e.bottons & 1 != 0);
        let is_down = next.bottons & 1 != 0;
        let changed_position = self
            .sent
            .as_ref()
            .is_some_and(|e| (e.position_x, e.position_y) != (next.position_x, next.position_y));
        if was_down
            && !is_down
            && !self.dragged
            && !changed_position
            && self.down_at.is_some_and(|at| now < at + MIN_CLICK_HOLD)
        {
            return None;
        }
        let event = self.pending.pop_front()?;
        if is_down && !was_down {
            // Start from actual dispatch, not enqueue time: a busy network loop
            // must not compress a queued down/up pair into a zero-length click.
            self.down_at = Some(now);
            self.dragged = false;
        } else if is_down && changed_position {
            self.dragged = true;
        } else if !is_down {
            self.down_at = None;
        }
        self.sent = Some(event.clone());
        Some(event)
    }

    pub fn cancel(&mut self) -> Option<ClientMouseEvent> {
        self.pending.clear();
        self.down_at = None;
        let mut event = self.sent.take()?;
        if event.bottons == 0 {
            return None;
        }
        event.bottons = 0;
        Some(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quick_click_waits_and_hover_does_not_overtake_release() {
        let mut timing = PointerTiming::default();
        let now = Instant::now();
        timing.push((10, 20, 1).into());
        timing.push((10, 20, 0).into());
        timing.push((90, 80, 0).into());
        assert_eq!(timing.pop_ready(now).unwrap().bottons, 1);
        assert!(timing.pop_ready(now + Duration::from_millis(99)).is_none());
        let up = timing.pop_ready(now + MIN_CLICK_HOLD).unwrap();
        assert_eq!((up.position_x, up.position_y, up.bottons), (10, 20, 0));
        assert_eq!(
            timing.pop_ready(now + MIN_CLICK_HOLD).unwrap().position_x,
            90
        );
    }

    #[test]
    fn consecutive_clicks_each_get_a_hold_from_actual_dispatch() {
        let mut timing = PointerTiming::default();
        let now = Instant::now();
        for _ in 0..2 {
            timing.push((10, 20, 1).into());
            timing.push((10, 20, 0).into());
        }
        // Dispatch starts late; enqueue time must not count toward the hold.
        let late = now + Duration::from_secs(1);
        timing.pop_ready(late).unwrap();
        assert!(timing.pop_ready(late).is_none());
        assert_eq!(timing.pop_ready(late + MIN_CLICK_HOLD).unwrap().bottons, 0);
        assert_eq!(timing.pop_ready(late + MIN_CLICK_HOLD).unwrap().bottons, 1);
        assert!(timing.pop_ready(late + MIN_CLICK_HOLD).is_none());
        assert_eq!(
            timing.pop_ready(late + MIN_CLICK_HOLD * 2).unwrap().bottons,
            0
        );
    }

    #[test]
    fn long_press_drag_and_non_left_buttons_are_not_delayed() {
        let now = Instant::now();
        for drag in [false, true] {
            let mut timing = PointerTiming::default();
            timing.push((10, 20, 1).into());
            timing.pop_ready(now).unwrap();
            if drag {
                timing.push((30, 40, 1).into());
                timing.pop_ready(now).unwrap();
                timing.push((30, 40, 0).into());
                assert!(timing.pop_ready(now).is_some());
            } else {
                timing.push((10, 20, 0).into());
                assert!(timing.pop_ready(now + Duration::from_secs(1)).is_some());
            }
        }
        let mut timing = PointerTiming::default();
        for buttons in [2, 0, 4, 0, 8, 0, 16, 0] {
            timing.push((10, 20, buttons).into());
            assert_eq!(timing.pop_ready(now).unwrap().bottons, buttons);
        }
        timing.push((10, 20, 1).into());
        timing.pop_ready(now).unwrap();
        timing.push((11, 20, 0).into()); // Drag visible only in the up packet.
        assert!(timing.pop_ready(now).is_some());
    }

    #[test]
    fn close_discards_queued_clicks_and_releases_only_sent_buttons() {
        let now = Instant::now();
        let mut timing = PointerTiming::default();
        timing.push((10, 20, 1).into());
        assert!(timing.cancel().is_none());
        timing.push((10, 20, 1).into());
        timing.pop_ready(now).unwrap();
        timing.push((10, 20, 0).into());
        timing.push((99, 99, 1).into());
        let release = timing.cancel().unwrap();
        assert_eq!(
            (release.position_x, release.position_y, release.bottons),
            (10, 20, 0)
        );
        assert!(timing.pop_ready(now + MIN_CLICK_HOLD).is_none());
        assert!(timing.cancel().is_none());
    }
}
