//! Extraction policies for the independent mailbox tool, unrelated to phone tasks.
mod generic;
mod vinted;

pub(super) struct Message<'a> {
    pub sender: &'a str,
    pub subject: &'a str,
    pub body: &'a str,
}

trait CodeStrategy: Sync {
    fn matches(&self, message: &Message<'_>) -> bool;
    fn extract(&self, message: &Message<'_>) -> Option<String>;
}

const STRATEGIES: &[&dyn CodeStrategy] = &[&vinted::Vinted, &generic::Generic];

pub(super) fn extract(sender: &str, subject: &str, body: &str) -> Option<String> {
    let message = Message {
        sender,
        subject,
        body,
    };
    STRATEGIES
        .iter()
        .filter(|strategy| strategy.matches(&message))
        .find_map(|strategy| strategy.extract(&message))
}

#[cfg(test)]
mod tests {
    use super::extract;

    #[test]
    fn handles_other_brands_lengths_and_alphanumeric_codes() {
        for (body, expected) in [
            ("Your verification code: 0042", "0042"),
            ("Your security code is 012345", "012345"),
            ("Your one-time code: 00123456", "00123456"),
            ("验证码：008877", "008877"),
            ("验证码是0042，请勿泄露", "0042"),
            ("验证码：0042请勿泄露", "0042"),
            ("Your verification code: AB12CD", "AB12CD"),
            ("OTP: 0042", "0042"),
            ("Code: A12B34", "A12B34"),
            ("Your 8-digit code is 00123456", "00123456"),
            ("0427", "0427"),
        ] {
            assert_eq!(
                extract("sender@example.test", "Account verification", body).as_deref(),
                Some(expected)
            );
        }
    }

    #[test]
    fn reads_subject_codes_and_prefers_vinted_login_format() {
        assert_eq!(
            extract(
                "service@example.test",
                "Your verification code: 0042",
                "Account notice"
            )
            .as_deref(),
            Some("0042")
        );
        assert_eq!(extract("no-reply_at_vinted_fr_alias_hash@privaterelay.appleid.com", "New login attempt",
            "Reference 123456. Enter this 4-digit code to verify that it's you: <b>0427</b>. Valid for 15 minutes.").as_deref(), Some("0427"));
    }

    #[test]
    fn ignores_unrelated_years_orders_and_expiry_minutes() {
        assert_eq!(
            extract(
                "sender@example.test",
                "Account verification",
                "Your verification code expires in 15 minutes. Order 12345678 ships tomorrow."
            ),
            None
        );
        assert_eq!(
            extract(
                "sender@example.test",
                "Order 2026",
                "Order 12345678, delivery in 15 minutes"
            ),
            None
        );
        assert_eq!(
            extract(
                "sender@example.test",
                "Account verification",
                "Your verification code expires in 15 minutes."
            ),
            None
        );
    }
}
