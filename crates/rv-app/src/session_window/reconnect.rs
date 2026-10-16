use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct Recovery {
    pub visible: bool,
    pub stopped: bool,
    pub attempt: u32,
    pub deadline: Option<Instant>,
}
impl Recovery {
    pub fn connected(&mut self) {
        self.visible = false;
        self.attempt = 0;
        self.deadline = None;
    }
    pub fn schedule(&mut self, error: Option<&str>, now: Instant) {
        self.deadline = None;
        self.visible = true;
        if self.stopped || !retryable(error) {
            return;
        }
        self.attempt = self.attempt.saturating_add(1);
        self.deadline = Some(now + retry_delay(self.attempt));
    }
    pub fn seconds(&self, now: Instant) -> u64 {
        self.deadline.map_or(0, |deadline| {
            let remaining = deadline.saturating_duration_since(now);
            remaining.as_secs() + u64::from(remaining.subsec_nanos() > 0)
        })
    }
    pub fn stop(&mut self) {
        self.stopped = true;
        self.deadline = None;
    }
}
/// Exponential backoff starts at three seconds and caps at one minute.
/// Clamp before shifting so even a saturated attempt counter remains safe.
fn retry_delay(attempt: u32) -> Duration {
    let exponent = attempt.saturating_sub(1).min(5);
    Duration::from_secs((3_u64 << exponent).min(60))
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
        "connection refused",
        "connection aborted",
        "network is unreachable",
        "network unreachable",
        "no route to host",
        "host is down",
        "operation timed out",
        "temporary failure in name resolution",
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
    fn retries_continue_beyond_five_and_backoff_is_capped_until_connected() {
        let now = Instant::now();
        let mut recovery = Recovery::default();
        recovery.connected();
        for attempt in 1..=20 {
            recovery.schedule(Some("server closed the connection"), now);
            assert_eq!(recovery.attempt, attempt);
            let expected = [3, 6, 12, 24, 48, 60][attempt.saturating_sub(1).min(5) as usize];
            assert_eq!(recovery.deadline, Some(now + Duration::from_secs(expected)));
        }
        assert!(recovery.visible);
        recovery.connected();
        assert!(!recovery.visible);
        assert_eq!(recovery.attempt, 0);
        assert!(recovery.deadline.is_none());
        recovery.schedule(None, now);
        assert_eq!(recovery.deadline, Some(now + Duration::from_secs(3)));
        recovery.attempt = u32::MAX;
        recovery.schedule(None, now);
        assert_eq!(recovery.deadline, Some(now + Duration::from_secs(60)));
    }
    #[test]
    fn initial_connection_refusal_retries_and_respects_stop_and_terminal_errors() {
        let now = Instant::now();
        let mut recovery = Recovery::default();
        recovery.schedule(
            Some("cannot connect to 172.16.19.55:5901: Connection refused (os error 61)"),
            now,
        );
        assert_eq!(recovery.deadline, Some(now + Duration::from_secs(3)));
        assert_eq!(recovery.seconds(now), 3);
        assert_eq!(recovery.seconds(now + Duration::from_millis(500)), 3);
        assert_eq!(recovery.seconds(now + Duration::from_secs(3)), 0);
        recovery.schedule(Some("connection timed out"), now);
        assert_eq!(recovery.deadline, Some(now + Duration::from_secs(6)));
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
