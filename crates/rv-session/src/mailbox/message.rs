//! Shared message validation, independent of a mailbox transport.

pub(super) fn apple_sender(address: &str) -> bool {
    address.rsplit_once('@').is_some_and(|(_, domain)| {
        let domain = domain.to_ascii_lowercase();
        domain == "apple.com" || domain.ends_with(".apple.com")
    })
}
pub(super) fn code(subject: &str, body: &str) -> Result<Option<String>, String> {
    // Called only after the provider verifies sender, recipient and freshness.
    // The phone's pending email challenge scopes this read; message language
    // and localized purpose keywords must not prevent extracting its code.
    let plain = plain_text(&format!("{subject} {body}"));
    Ok(plain
        .split(|c: char| !c.is_ascii_digit())
        .find(|s| s.len() == 6 && s.bytes().all(|b| b.is_ascii_digit()))
        .map(str::to_owned))
}

pub(super) fn plain_text(text: &str) -> String {
    let mut plain = String::new();
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => {
                in_tag = true;
                plain.push(' ');
            }
            '>' => {
                in_tag = false;
                plain.push(' ');
            }
            _ if !in_tag => plain.push(c),
            _ => (),
        }
    }
    plain
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extracts_first_code_and_rejects_false_sender() {
        assert!(apple_sender("appleid@id.apple.com"));
        assert!(!apple_sender("a@apple.com.evil.test"));
        assert_eq!(
            code("Verify your email", "Your verification code: <b>012345</b>"),
            Ok(Some("012345".into()))
        );
        assert_eq!(code("Apple", "012345"), Ok(Some("012345".into())));
        assert_eq!(
            code("Verify email", "123456 or 654321"),
            Ok(Some("123456".into()))
        );
        assert_eq!(code("Verify email", "reference 1234567"), Ok(None));
    }

    #[test]
    fn traditional_chinese_contact_email_verification() {
        assert_eq!(
            code("驗證你的電子郵件地址", "請輸入驗證碼：<b>012345</b>"),
            Ok(Some("012345".into()))
        );
        assert_eq!(
            code("Apple", "你的驗證碼是 012345"),
            Ok(Some("012345".into()))
        );
        assert_eq!(
            code("驗證你的電子郵件地址", "123456 或 654321"),
            Ok(Some("123456".into()))
        );
    }

    #[test]
    fn contact_verification_in_multiple_languages() {
        // Representative localized inputs, with synthetic addresses/codes.
        for subject in [
            "Verify your email address",
            "验证你的电子邮件地址",
            "驗證你的電子郵件地址",
            "メールアドレスを確認してください",
            "이메일 주소 확인",
            "Vérifiez votre adresse e-mail",
            "Bestätige deine E-Mail-Adresse",
            "Verifica tu dirección de correo electrónico",
            "Verifique seu endereço de e-mail",
            "Verifica il tuo indirizzo email",
            "Bevestig je e-mailadres",
            "Подтвердите адрес электронной почты",
            "Підтвердьте адресу електронної пошти",
            "Potwierdź swój adres e-mail",
            "E-posta adresinizi doğrulayın",
            "Xác minh địa chỉ email của bạn",
            "Verifikasi alamat email Anda",
            "ยืนยันที่อยู่อีเมลของคุณ",
            "تأكيد عنوان البريد الإلكتروني",
            "ईमेल पते की पुष्टि करें",
            "Bekräfta din e-postadress",
            "Vahvista sähköpostiosoitteesi",
        ] {
            assert_eq!(
                code(subject, "<b>012345</b>"),
                Ok(Some("012345".into())),
                "{subject}"
            );
        }
    }

    #[test]
    fn extraction_does_not_require_language_or_purpose_keywords() {
        assert_eq!(code("Apple", "<b>012345</b>"), Ok(Some("012345".into())));
        assert_eq!(code("012345", "Apple"), Ok(Some("012345".into())));
        assert_eq!(code("012345", "012345"), Ok(Some("012345".into())));
        assert_eq!(code("012345", "654321"), Ok(Some("012345".into())));
        assert_eq!(code("Apple", "1234567"), Ok(None));
        assert_eq!(code("Apple", "No number"), Ok(None));
    }
}
