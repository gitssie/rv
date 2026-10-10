use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct Recovery {
    pub established: bool,
    pub visible: bool,
    pub stopped: bool,
    pub attempt: u32,
    pub deadline: Option<Instant>,
}
impl Recovery {
    pub fn connected(&mut self) {
        self.established = true;
        self.visible = false;
        self.attempt = 0;
        self.deadline = None;
    }
    pub fn schedule(&mut self, error: Option<&str>, now: Instant) {
        self.deadline = None;
        self.visible = true;
        if self.attempt >= 5 || !self.established || self.stopped || !retryable(error) {
            return;
        }
        self.attempt = self.attempt.saturating_add(1);
        let delay = [3, 5, 10, 20, 30][self.attempt.saturating_sub(1).min(4) as usize];
        self.deadline = Some(now + Duration::from_secs(delay));
    }
    pub fn seconds(&self, now: Instant) -> u64 {
        self.deadline.map_or(0, |deadline| {
            deadline
                .saturating_duration_since(now)
                .as_secs()
                .saturating_add(1)
        })
    }
    pub fn stop(&mut self) {
        self.stopped = true;
        self.deadline = None;
    }
}
fn retryable(error: Option<&str>) -> bool {
    let Some(error) = error else {
        return true;
    };
    let error = error.to_ascii_lowercase();
    // Only retry transport failures; malformed protocol, TLS and rejected credentials stop.
    [
        "i/o error:",
        "connection timed out",
        "server closed the connection",
        "connection reset",
        "broken pipe",
        "unexpected eof",
        "unexpected end of file",
        "failed to fill whole buffer",
    ]
    .iter()
    .any(|reason| error.contains(reason))
        && !super::password_error(&error)
        && !error.contains("tls")
        && !error.contains("certificate")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retry_limit_preserves_dialog_until_connected() {
        let now = Instant::now();
        let mut recovery = Recovery::default();
        recovery.connected();
        for attempt in 1..=5 {
            recovery.schedule(Some("server closed the connection"), now);
            assert_eq!(recovery.attempt, attempt);
            assert!(recovery.deadline.is_some());
        }
        recovery.schedule(Some("server closed the connection"), now);
        assert_eq!(recovery.attempt, 5);
        assert!(recovery.deadline.is_none());
        assert!(recovery.visible);
        recovery.connected();
        assert!(!recovery.visible);
    }
    #[test]
    fn recovery_only_follows_established_transport_failures_and_respects_stop() {
        let now = Instant::now();
        let mut recovery = Recovery::default();
        recovery.schedule(None, now);
        assert!(recovery.deadline.is_none());
        recovery.connected();
        recovery.schedule(Some("I/O error: Connection reset"), now);
        assert_eq!(recovery.deadline, Some(now + Duration::from_secs(3)));
        recovery.schedule(Some("connection timed out"), now);
        assert_eq!(recovery.deadline, Some(now + Duration::from_secs(5)));
        for error in [
            "authentication failed",
            "TLS error: certificate rejected",
            "Invalid device reply header",
        ] {
            recovery.schedule(Some(error), now);
            assert!(recovery.deadline.is_none());
        }
        recovery.stop();
        recovery.schedule(None, now);
        assert!(recovery.deadline.is_none());
    }
}
