//! Only the French email challenge and phone gate observed on 2026-10-28.
use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Email,
    PhoneRequired,
    Other,
}

pub fn classify(observed: &Observation, email: &str) -> Page {
    if !valid_page(observed) {
        return Page::Other;
    }
    if has(observed, "Ajouter mon numéro de téléphone")
        && observed
            .elements
            .iter()
            .any(|e| e.text.contains("sécuriser ton compte"))
    {
        return Page::PhoneRequired;
    }
    if has(observed, email)
        && has(observed, "Vérifier")
        && observed
            .elements
            .iter()
            .any(|e| e.text.contains("Vérifie ton adresse e-mail"))
        && has(observed, "Saisis le code de vérification ici")
    {
        return Page::Email;
    }
    Page::Other
}

pub fn enter_code(
    observed: &Observation,
    email: &str,
    code: &str,
) -> Result<Workflow, RegistrationError> {
    if classify(observed, email) != Page::Email
        || code.len() != 6
        || !code.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(RegistrationError::WrongPage);
    }
    Ok(plan(vec![
        tap("Saisis le code de vérification ici"),
        Action::Delay { milliseconds: 1200 },
        Action::InputText {
            input: "verification_code".into(),
        },
        Action::WaitForInput {
            input: "verification_code".into(),
            case_sensitive: true,
            region: Some(Rect {
                x: 0.,
                y: 0.2,
                width: 1.,
                height: 0.15,
            }),
        },
        tap("Vérifier"),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rv_automation::TextElement;
    fn observed() -> Observation {
        Observation {
            frame: Frame {
                width: 1170,
                height: 2532,
                orientation: 0,
            },
            bundle_id: Some(BUNDLE_ID.into()),
            elements: [
                "Se déconnecter Vérifie ton adresse e-mail",
                "fixture@example.test",
                "Saisis le code de vérification ici",
                "Vérifier",
            ]
            .into_iter()
            .enumerate()
            .map(|(i, text)| TextElement {
                text: text.into(),
                bounds: Rect {
                    x: 40.,
                    y: 180. + i as f64 * 180.,
                    width: 900.,
                    height: 50.,
                },
            })
            .collect(),
        }
    }
    #[test]
    fn challenge_requires_correct_app_email_empty_field_and_six_digits() {
        let mut o = observed();
        assert!(
            enter_code(&o, "fixture@example.test", "012345")
                .unwrap()
                .is_valid()
        );
        assert!(enter_code(&o, "other@example.test", "012345").is_err());
        for bad in ["12345", "1234567", "１２３４５６", "12x456"] {
            assert!(enter_code(&o, "fixture@example.test", bad).is_err());
        }
        o.elements[2].text = "012345".into();
        assert!(enter_code(&o, "fixture@example.test", "012345").is_err());
        o.bundle_id = Some("other.app".into());
        assert!(classify(&o, "fixture@example.test") == Page::Other);
    }
    #[test]
    fn phone_gate_is_pending_and_unknown_pages_are_never_success() {
        let mut o = observed();
        o.elements[0].text = "pour sécuriser ton compte".into();
        o.elements[1].text = "Ajouter mon numéro de téléphone".into();
        assert!(classify(&o, "fixture@example.test") == Page::PhoneRequired);
        o.elements.clear();
        assert!(classify(&o, "fixture@example.test") == Page::Other);
    }
}
