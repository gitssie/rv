use super::*;
use rv_automation::TextElement;

fn element(text: &str, y: f64) -> TextElement {
    TextElement {
        text: text.into(),
        bounds: Rect {
            x: 120.,
            y,
            width: 700.,
            height: 30.,
        },
    }
}
fn form() -> Observation {
    Observation {
        frame: Frame {
            width: 1000,
            height: 2000,
            orientation: 0,
        },
        bundle_id: Some(BUNDLE_ID.into()),
        elements: vec![
            element("Continuer", 100.),
            element("fixtureUser", 300.),
            element("fixture@example.invalid", 400.),
            element("••••••••••••••••", 500.),
            element("Je souhaite recevoir par e-mail des offres", 750.),
            element("de Vinted.", 850.),
            element("En t'inscrivant, tu confirmes", 950.),
            element("et avoir au moins 18 ans.", 1100.),
            element("Continuer", 1400.),
        ],
    }
}
fn values() -> InputValues {
    let mut inputs = InputValues::default();
    inputs.insert("username", "fixtureUser".into());
    inputs.insert("email", "fixture@example.invalid".into());
    inputs.insert("password", "fixture-password".into());
    inputs
}
fn picture(marketing: bool, terms: bool) -> RgbImage {
    let mut image = RgbImage::from_pixel(1000, 2000, image::Rgb([255, 255, 255]));
    for (checked, y) in [(marketing, 815), (terms, 1040)] {
        for row in y - 30..y + 30 {
            for column in 42..102 {
                let border = row < y - 27 || row >= y + 27 || !(45..99).contains(&column);
                let color = if checked {
                    [0, 120, 135]
                } else if border {
                    [110, 120, 120]
                } else {
                    [255, 255, 255]
                };
                image.put_pixel(column, row, image::Rgb(color));
            }
        }
    }
    image
}

#[test]
fn phases_are_executable_and_keep_all_customer_values_out_of_definitions() {
    let mut workflows = vec![open_registration(), choose_email()];
    for field in [Field::Username, Field::Email, Field::Password] {
        workflows.push(field_input(field));
        if let Some(verify) = verify_visible_field(field) {
            workflows.push(verify);
        }
    }
    for workflow in workflows {
        assert!(workflow.is_valid());
        assert_eq!(workflow.bundle_id, BUNDLE_ID);
        let encoded = serde_json::to_string(&workflow).unwrap();
        for private in ["fixtureUser", "fixture@example.invalid", "fixture-password"] {
            assert!(!encoded.contains(private));
        }
    }
    assert!(verify_visible_field(Field::Password).is_none());
}

#[test]
fn paste_authorization_is_scoped_to_both_apps_and_unique_native_button() {
    let mut observed = form();
    observed.elements.extend([
        element("« Vinted » souhaite coller un élément depuis", 800.),
        element("« trollvncserver »", 850.),
        element("Ne pas autoriser le collage", 1000.),
        element("Autoriser le collage", 1100.),
    ]);
    let workflow = owned_paste_permission(&observed).unwrap().unwrap();
    assert_eq!(workflow.steps.len(), 1);
    assert!(matches!(
        workflow.steps[0].action,
        Action::Tap {
            target: Target::FramePoint { .. }
        }
    ));
    observed.elements.last_mut().unwrap().text = "Something else".into();
    assert!(owned_paste_permission(&observed).unwrap().is_none());
    observed.elements.last_mut().unwrap().text = "Autoriser le collage".into();
    observed.elements[11].text = "« unrelated app »".into();
    assert!(owned_paste_permission(&observed).is_err());
}

#[test]
fn retains_marketing_opt_out_and_only_changes_terms_when_needed() {
    let current = inspect_checkboxes(&form(), &picture(false, false)).unwrap();
    assert!(!current.marketing && !current.terms);
    let preferences = Preferences {
        marketing: false,
        accept_terms: true,
    };
    let workflow = consent_changes(current, preferences).unwrap();
    assert_eq!(workflow.steps.len(), 1);
    assert!(
        matches!(workflow.steps[0].action,Action::Tap { target:Target::FramePoint { point,..} } if point==current.terms_point)
    );
    let checked = inspect_checkboxes(&form(), &picture(false, true)).unwrap();
    assert!(consent_changes(checked, preferences).is_none());
    let subscribed = inspect_checkboxes(&form(), &picture(true, true)).unwrap();
    let workflow = consent_changes(subscribed, preferences).unwrap();
    assert!(
        matches!(workflow.steps[0].action,Action::Tap { target:Target::FramePoint { point,..} } if point==subscribed.marketing_point)
    );
}

#[test]
fn empty_image_is_not_mistaken_for_two_unchecked_controls() {
    let blank = RgbImage::from_pixel(1000, 2000, image::Rgb([255, 255, 255]));
    assert!(matches!(
        inspect_checkboxes(&form(), &blank),
        Err(RegistrationError::UnknownCheckbox)
    ));
    let wrong_size = RgbImage::new(2000, 1000);
    assert!(inspect_checkboxes(&form(), &wrong_size).is_err());
}

#[test]
fn submit_requires_terms_correct_fields_and_a_unique_body_button() {
    let inputs = values();
    let password = confirm_password(&form(), &inputs).unwrap();
    let preferences = Preferences {
        marketing: false,
        accept_terms: true,
    };
    let current = inspect_checkboxes(&form(), &picture(false, true)).unwrap();
    let workflow = submit(&form(), current, preferences, &inputs, &password).unwrap();
    assert_eq!(workflow.steps.len(), 1);
    assert!(
        matches!(workflow.steps[0].action,Action::Tap { target:Target::FramePoint { point,..} } if point.y==1415.)
    );
    let unchecked = inspect_checkboxes(&form(), &picture(false, false)).unwrap();
    assert!(matches!(
        submit(&form(), unchecked, preferences, &inputs, &password),
        Err(RegistrationError::TermsNotAccepted)
    ));
    let mut wrong = form();
    wrong.elements[2].text = "another@example.invalid".into();
    assert!(submit(&wrong, current, preferences, &inputs, &password).is_err());
    let mut empty_password = form();
    empty_password.elements[3].text = Field::Password.label().into();
    assert!(submit(&empty_password, current, preferences, &inputs, &password).is_err());
    let mut duplicate = form();
    duplicate.elements.push(element("Continuer", 1500.));
    assert!(matches!(
        submit(&duplicate, current, preferences, &inputs, &password),
        Err(RegistrationError::AmbiguousTarget)
    ));
}

#[test]
fn missing_password_ocr_is_not_positive_password_evidence() {
    let mut missing = form();
    missing.elements.remove(3);
    assert!(matches!(
        confirm_password(&missing, &values()),
        Err(RegistrationError::PasswordNotConfirmed)
    ));
    let mut inputs = InputValues::default();
    inputs.insert("username", "fixtureUser".into());
    inputs.insert("email", "fixture@example.invalid".into());
    assert!(matches!(
        confirm_password(&form(), &inputs),
        Err(RegistrationError::PasswordNotConfirmed)
    ));
}

#[test]
fn owned_password_save_prompt_is_positive_evidence_and_only_declined() {
    let mut observed = form();
    observed.elements.remove(3);
    observed.elements.extend([
        element(
            "Voulez-vous enregistrer ce mot de passe dans votre trousseau",
            1500.,
        ),
        element("Enregistrer le mot de passe", 1700.),
        element("Plus tard", 1800.),
    ]);
    assert!(confirm_password(&observed, &values()).is_ok());
    let workflow = decline_password_save(&observed).unwrap().unwrap();
    assert!(
        matches!(workflow.steps[0].action,Action::Tap { target:Target::FramePoint { point,..} } if point.y==1815.)
    );
    observed.bundle_id = Some("unrelated.app".into());
    assert!(confirm_password(&observed, &values()).is_err());
}

fn dispatch_replies() -> Vec<crate::automation::tests::Expected> {
    use crate::automation::tests::{Expected, response};
    use serde_json::json;
    let mut replies = Vec::new();
    let observe = || {
        vec![
            response("/api/apps/foreground", json!({"bundle_id":BUNDLE_ID})),
            response(
                "/api/device/ocr",
                json!({"value":"Nom d'utilisateur,,100,,350,,300,,40","frame":{"width":1000,"height":2000,"orientation":0}}),
            ),
            response("/api/apps/foreground", json!({"bundle_id":BUNDLE_ID})),
        ]
    };
    replies.extend(observe());
    replies.extend(observe());
    replies.push(response("/api/device/tap", json!({"ok":true})));
    replies.extend(observe());
    replies.push(response(
        "/api/control/authorize",
        json!({"keyboard_literal":1,"device_tap_hold":1}),
    ));
    replies.push(response(
        "/api/apps/foreground",
        json!({"bundle_id":BUNDLE_ID}),
    ));
    replies.push(Expected {
        body: Some(json!({"fields":["8","fixtureUser"]})),
        ..response("/api/device/keyboard", json!({"value":""}))
    });
    replies.extend(crate::automation::tests::post_paste_replies(BUNDLE_ID));
    replies
}

#[tokio::test]
async fn postdispatch_ocr_failure_retains_pending_insertion_status() {
    use crate::automation::tests::{Expected, fixture, response};
    use serde_json::json;
    let mut replies = dispatch_replies();
    replies.push(Expected {
        status: 503,
        ..response("/api/apps/foreground", json!({"error":"unavailable"}))
    });
    let (device, calls, _, server) = fixture(replies).await;
    let result = fill_field(
        &device.client,
        Field::Username,
        &values(),
        &RunControl::default(),
    )
    .await
    .unwrap_err();
    assert!(result.input_may_be_pending());
    assert!(matches!(
        result,
        RegistrationError::InputMayBePending { .. }
    ));
    assert_eq!(
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|path| path.as_str() == "/api/device/keyboard")
            .count(),
        1
    );
    server.abort();
}

#[tokio::test]
async fn cancelling_permission_observation_does_not_erase_the_previous_pending_paste() {
    use crate::automation::tests::{Expected, fixture, response};
    use serde_json::json;
    let mut replies = dispatch_replies();
    replies.extend([
        response("/api/apps/foreground",json!({"bundle_id":BUNDLE_ID})),
        response("/api/device/ocr",json!({"value":"« Vinted » souhaite coller un élément depuis,,100,,800,,700,,40;;« trollvncserver »,,100,,850,,700,,40;;Ne pas autoriser le collage,,100,,1000,,700,,40;;Autoriser le collage,,100,,1100,,700,,40","frame":{"width":1000,"height":2000,"orientation":0}})),
        response("/api/apps/foreground",json!({"bundle_id":BUNDLE_ID})),
    ]);
    let started = std::sync::Arc::new(tokio::sync::Notify::new());
    let resume = std::sync::Arc::new(tokio::sync::Notify::new());
    replies.push(Expected {
        pause: Some((started.clone(), resume.clone())),
        ..response("/api/apps/foreground", json!({"bundle_id":BUNDLE_ID}))
    });
    let (device, calls, _, server) = fixture(replies).await;
    let control = RunControl::default();
    let inputs = values();
    let (result, ()) = tokio::join!(
        fill_field(&device.client, Field::Username, &inputs, &control),
        async {
            started.notified().await;
            control.cancel();
            resume.notify_one();
        }
    );
    let error = result.unwrap_err();
    assert!(error.input_may_be_pending());
    assert!(
        matches!(error,RegistrationError::InputMayBePending { diagnostic } if matches!(*diagnostic,RegistrationError::Execution(Progress { state:RunState::Cancelled,.. })))
    );
    let calls = calls.lock().unwrap();
    assert_eq!(
        calls
            .iter()
            .filter(|path| path.as_str() == "/api/device/tap")
            .count(),
        3 // Focus tap, long hold and short tap; no permission tap after cancellation.
    );
    assert_eq!(
        calls
            .iter()
            .filter(|path| path.as_str() == "/api/device/keyboard")
            .count(),
        1
    );
    server.abort();
}
