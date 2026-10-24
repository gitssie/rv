use std::collections::VecDeque;
use std::time::{Duration, Instant};

use vnc::ClientMouseEvent;

const MIN_CLICK_HOLD: Duration = Duration::from_millis(60);
const MAX_CLICK_HOLD: Duration = Duration::from_millis(200);

fn random_click_hold() -> Duration {
    // Rejection sampling gives every integer millisecond in 60..=200 the
    // same probability. UUID byte 0 contains no fixed version/variant bits.
    loop {
        let sample = uuid::Uuid::new_v4().as_bytes()[0];
        if u128::from(sample) <= (MAX_CLICK_HOLD - MIN_CLICK_HOLD).as_millis() {
            return MIN_CLICK_HOLD + Duration::from_millis(u64::from(sample));
        }
    }
}

/// Preserve pointer ordering with a fresh 60–200 ms minimum hold per left click.
/// Longer holds and drags retain their timing. Keys, frames and shutdown do not wait.
#[derive(Default)]
pub(crate) struct PointerTiming {
    pending: VecDeque<ClientMouseEvent>,
    sent: Option<ClientMouseEvent>,
    release_at: Option<Instant>,
    dragged: bool,
}

impl PointerTiming {
    pub fn push(&mut self, event: ClientMouseEvent) {
        self.pending.push_back(event);
    }

    pub fn pop_ready(&mut self, now: Instant) -> Option<ClientMouseEvent> {
        self.pop_ready_with_hold(now, random_click_hold)
    }

    fn pop_ready_with_hold(
        &mut self,
        now: Instant,
        next_hold: impl FnOnce() -> Duration,
    ) -> Option<ClientMouseEvent> {
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
            && self.release_at.is_some_and(|at| now < at)
        {
            return None;
        }
        let event = self.pending.pop_front()?;
        if is_down && !was_down {
            // Start from actual dispatch, not enqueue time: a busy network loop
            // must not compress a queued down/up pair into a zero-length click.
            self.release_at = Some(now + next_hold());
            self.dragged = false;
        } else if is_down && changed_position {
            self.dragged = true;
        } else if !is_down {
            self.release_at = None;
        }
        self.sent = Some(event.clone());
        Some(event)
    }

    pub fn cancel(&mut self) -> Option<ClientMouseEvent> {
        self.pending.clear();
        self.release_at = None;
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
    fn each_press_samples_once_and_keeps_its_deadline_including_range_endpoints() {
        let mut timing = PointerTiming::default();
        let mut now = Instant::now();
        for hold in [MIN_CLICK_HOLD, MAX_CLICK_HOLD, Duration::from_millis(137)] {
            timing.push((10, 20, 1).into());
            timing.push((10, 20, 0).into());
            assert_eq!(timing.pop_ready_with_hold(now, || hold).unwrap().bottons, 1);
            let deadline = now + hold;
            for offset in [0, 1, 30] {
                assert!(
                    timing
                        .pop_ready_with_hold(now + Duration::from_millis(offset), || {
                            panic!("must not resample while polling release")
                        })
                        .is_none()
                );
            }
            assert!(
                timing
                    .pop_ready(deadline - Duration::from_millis(1))
                    .is_none()
            );
            assert_eq!(timing.pop_ready(deadline).unwrap().bottons, 0);
            now = deadline;
        }
    }

    #[test]
    fn random_holds_stay_in_requested_range() {
        for _ in 0..1000 {
            assert!((MIN_CLICK_HOLD..=MAX_CLICK_HOLD).contains(&random_click_hold()));
        }
    }

    #[test]
    fn quick_click_waits_and_hover_does_not_overtake_release() {
        let mut timing = PointerTiming::default();
        let now = Instant::now();
        timing.push((10, 20, 1).into());
        timing.push((10, 20, 0).into());
        timing.push((90, 80, 0).into());
        assert_eq!(timing.pop_ready(now).unwrap().bottons, 1);
        let deadline = timing.release_at.unwrap();
        assert!((MIN_CLICK_HOLD..=MAX_CLICK_HOLD).contains(&deadline.duration_since(now)));
        assert!(
            timing
                .pop_ready(deadline - Duration::from_millis(1))
                .is_none()
        );
        let up = timing.pop_ready(deadline).unwrap();
        assert_eq!((up.position_x, up.position_y, up.bottons), (10, 20, 0));
        assert_eq!(timing.pop_ready(deadline).unwrap().position_x, 90);
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
        let first_deadline = timing.release_at.unwrap();
        assert!(
            timing
                .pop_ready(first_deadline - Duration::from_millis(1))
                .is_none()
        );
        assert_eq!(timing.pop_ready(first_deadline).unwrap().bottons, 0);
        assert_eq!(timing.pop_ready(first_deadline).unwrap().bottons, 1);
        let second_deadline = timing.release_at.unwrap();
        assert!(
            (MIN_CLICK_HOLD..=MAX_CLICK_HOLD)
                .contains(&second_deadline.duration_since(first_deadline))
        );
        assert!(
            timing
                .pop_ready(second_deadline - Duration::from_millis(1))
                .is_none()
        );
        assert_eq!(timing.pop_ready(second_deadline).unwrap().bottons, 0);
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
