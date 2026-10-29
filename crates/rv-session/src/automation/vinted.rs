//! Observed French Vinted registration policy. Transport and the workflow engine
//! remain generic. These phases do not imply account creation or verification.
use image::RgbImage;
use rv_automation::{
    Action, Device, DeviceError, Frame, InputValues, Observation, Point, Progress, Rect,
    RunControl, RunState, Runner, Step, Target, TextMatch, TextSelector, Workflow,
};
use std::time::Duration;

use super::ZxTouchDevice;
use crate::phone_http::PhoneHttpClient;

pub mod verification;

pub const BUNDLE_ID: &str = "lt.manodrabuziai.fr";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Username,
    Email,
    Password,
}
impl Field {
    pub fn key(self) -> &'static str {
        match self {
            Self::Username => "username",
            Self::Email => "email",
            Self::Password => "password",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Username => "Nom d'utilisateur",
            Self::Email => "Adresse email",
            Self::Password => "Mot de passe",
        }
    }
}

/// Explicit user choices. There is deliberately no default accepting terms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Preferences {
    pub marketing: bool,
    pub accept_terms: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum RegistrationError {
    #[error("the phone is not on the expected Vinted page")]
    WrongPage,
    #[error("a target is absent or ambiguous")]
    AmbiguousTarget,
    #[error("checkbox appearance could not be confirmed")]
    UnknownCheckbox,
    #[error("registration terms have not been accepted")]
    TermsNotAccepted,
    #[error("a workflow step did not complete; inspect its structured outcome")]
    Execution(Progress),
    #[error("device operation failed")]
    Device(#[from] DeviceError),
    #[error("input may be pending; inspect the field before another attempt")]
    UnconfirmedInput,
    #[error("cancelled after input dispatch; input may be pending")]
    CancelledAfterDispatch,
    #[error("the insertion may still be pending; inspect it before another attempt")]
    InputMayBePending { diagnostic: Box<RegistrationError> },
    #[error("nonempty password entry has not been positively confirmed")]
    PasswordNotConfirmed,
}
impl RegistrationError {
    pub fn input_may_be_pending(&self) -> bool {
        matches!(
            self,
            Self::InputMayBePending { .. } | Self::UnconfirmedInput | Self::CancelledAfterDispatch
        ) || matches!(self,Self::Execution(progress) if progress.state==RunState::OutcomeUnknown)
    }
}

/// Password dispatch is explicitly separate from confirmed visible input.
#[derive(Debug)]
pub enum FieldOutcome {
    Verified(Progress),
    PasswordDispatched(Progress),
}

fn selector(text: &str, mode: TextMatch, region: Option<Rect>) -> TextSelector {
    TextSelector {
        text: text.into(),
        mode,
        case_sensitive: true,
        region,
    }
}
fn tap(text: &str) -> Action {
    Action::Tap {
        target: Target::Text {
            selector: selector(text, TextMatch::Exact, None),
            offset: Point { x: 0., y: 0. },
        },
    }
}
fn wait(text: &str) -> Action {
    Action::WaitForText {
        selector: selector(text, TextMatch::Exact, None),
        present: true,
    }
}
fn plan(actions: Vec<Action>) -> Workflow {
    Workflow {
        version: 1,
        bundle_id: BUNDLE_ID.into(),
        steps: actions
            .into_iter()
            .enumerate()
            .map(|(index, action)| Step {
                id: format!("vinted_{index}"),
                timeout_ms: 15_000,
                action,
            })
            .collect(),
    }
}
fn valid_page(observed: &Observation) -> bool {
    observed.is_valid()
        && observed.bundle_id.as_deref() == Some(BUNDLE_ID)
        && observed.frame.orientation == 0
}
fn has(observed: &Observation, text: &str) -> bool {
    observed
        .elements
        .iter()
        .any(|element| element.text.trim() == text)
}

pub fn open_registration() -> Workflow {
    let region = Some(Rect {
        x: 0.,
        y: 0.65,
        width: 1.,
        height: 0.3,
    });
    plan(vec![
        Action::WaitForText {
            selector: selector("S'inscrire sur Vinted", TextMatch::Exact, region),
            present: true,
        },
        Action::Tap {
            target: Target::Text {
                selector: selector("S'inscrire sur Vinted", TextMatch::Exact, region),
                offset: Point { x: 0., y: 0. },
            },
        },
        wait("Continuer avec une adresse e-mail"),
    ])
}
pub fn choose_email() -> Workflow {
    plan(vec![
        wait("Continuer avec une adresse e-mail"),
        tap("Continuer avec une adresse e-mail"),
        wait(Field::Username.label()),
        wait(Field::Email.label()),
        wait(Field::Password.label()),
    ])
}

/// Dispatch once, then reconcile the actual field/paste permission with
/// `fill_field`. Do not replay this workflow after an uncertain result.
pub fn field_input(field: Field) -> Workflow {
    plan(vec![
        wait(field.label()),
        tap(field.label()),
        // Native tap acceptance is earlier than UIKit keyboard/focus settling.
        Action::Delay { milliseconds: 1200 },
        Action::InputText {
            input: field.key().into(),
        },
    ])
}

pub fn verify_visible_field(field: Field) -> Option<Workflow> {
    if field == Field::Password {
        return None;
    }
    Some(plan(vec![Action::WaitForInput {
        input: field.key().into(),
        case_sensitive: field == Field::Username,
        region: Some(Rect {
            x: 0.,
            y: 0.1,
            width: 1.,
            height: 0.55,
        }),
    }]))
}

/// Only an explicitly pending registration insertion may authorize this prompt.
/// Both source and destination plus the allow/deny pair must be recognizable.
pub fn owned_paste_permission(
    observed: &Observation,
) -> Result<Option<Workflow>, RegistrationError> {
    if !valid_page(observed) {
        return Err(RegistrationError::WrongPage);
    }
    let text = observed
        .elements
        .iter()
        .map(|element| element.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    if !text.contains("Autoriser le collage") {
        return Ok(None);
    }
    if !text.contains("« Vinted »")
        || !text.contains("« trollvncserver »")
        || !text.contains("pas autoriser le collage")
    {
        return Err(RegistrationError::WrongPage);
    }
    let buttons: Vec<_> = observed
        .elements
        .iter()
        .filter(|element| element.text.trim() == "Autoriser le collage")
        .collect();
    if buttons.len() != 1 {
        return Err(RegistrationError::AmbiguousTarget);
    }
    Ok(Some(plan(vec![Action::Tap {
        target: Target::FramePoint {
            frame: observed.frame,
            point: buttons[0].bounds.center(),
        },
    }])))
}

async fn execute(
    client: &PhoneHttpClient,
    workflow: &Workflow,
    inputs: &InputValues,
    control: &RunControl,
) -> Result<Progress, RegistrationError> {
    let result = Runner::new(ZxTouchDevice::new(client.clone()))
        .run(workflow, inputs, control)
        .await;
    if result.state != RunState::Succeeded {
        return Err(RegistrationError::Execution(result));
    }
    Ok(result)
}

/// Caller holds the phone's exclusive input lease and provides its existing
/// authenticated client. No login/disconnect occurs here. A paste prompt is
/// acknowledged at most once for this insertion; input itself is never retried.
/// Password returns dispatch completion after its empty placeholder disappears;
/// this is masked-field evidence, not proof of its exact contents/account success.
pub async fn fill_field(
    client: &PhoneHttpClient,
    field: Field,
    inputs: &InputValues,
    control: &RunControl,
) -> Result<FieldOutcome, RegistrationError> {
    let dispatched = execute(client, &field_input(field), inputs, control).await?;
    reconcile_field(client, field, inputs, control, dispatched)
        .await
        .map_err(|diagnostic| RegistrationError::InputMayBePending {
            diagnostic: Box::new(diagnostic),
        })
}

async fn reconcile_field(
    client: &PhoneHttpClient,
    field: Field,
    inputs: &InputValues,
    control: &RunControl,
    dispatched: Progress,
) -> Result<FieldOutcome, RegistrationError> {
    let mut device = ZxTouchDevice::new(client.clone());
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let mut allowed = false;
    loop {
        let observed = tokio::select! {
            biased;
            _=control.cancelled()=>return Err(RegistrationError::CancelledAfterDispatch),
            _=tokio::time::sleep_until(deadline)=>return Err(RegistrationError::UnconfirmedInput),
            result=device.observe()=>result?,
        };
        if let Some(permission) = owned_paste_permission(&observed)? {
            if !allowed {
                execute(client, &permission, inputs, control).await?;
                allowed = true;
            }
        } else if field == Field::Password {
            if !has(&observed, field.label())
                && has(&observed, "Continuer")
                && [Field::Username, Field::Email].iter().all(|field| {
                    inputs
                        .get(field.key())
                        .is_some_and(|expected| has(&observed, expected))
                })
                && observed
                    .elements
                    .iter()
                    .any(|element| element.text.starts_with("Je souhaite recevoir"))
            {
                return Ok(FieldOutcome::PasswordDispatched(dispatched));
            }
        } else if inputs
            .get(field.key())
            .is_some_and(|expected| has(&observed, expected))
        {
            return execute(
                client,
                &verify_visible_field(field).expect("visible field"),
                inputs,
                control,
            )
            .await
            .map(FieldOutcome::Verified);
        }
        tokio::select! {
            biased;
            _=control.cancelled()=>return Err(RegistrationError::CancelledAfterDispatch),
            _=tokio::time::sleep(Duration::from_millis(250))=>{}
        }
    }
}

/// Opaque positive evidence, bound to immutable runtime inputs. It proves
/// nonempty password entry, not plaintext equality or account creation.
#[derive(Debug)]
pub struct PasswordEvidence<'a> {
    frame: Frame,
    inputs: &'a InputValues,
}

fn matching_identity(observed: &Observation, inputs: &InputValues) -> bool {
    [Field::Username, Field::Email].iter().all(|field| {
        inputs
            .get(field.key())
            .is_some_and(|value| has(observed, value))
    })
}

pub fn decline_password_save(
    observed: &Observation,
) -> Result<Option<Workflow>, RegistrationError> {
    if !valid_page(observed) {
        return Err(RegistrationError::WrongPage);
    }
    let combined = observed
        .elements
        .iter()
        .map(|element| element.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    if !combined.contains("trousseau") || !combined.contains("enregistrer ce mot de passe") {
        return Ok(None);
    }
    if !has(observed, "Enregistrer le mot de passe") {
        return Err(RegistrationError::AmbiguousTarget);
    }
    let buttons: Vec<_> = observed
        .elements
        .iter()
        .filter(|element| element.text.trim() == "Plus tard")
        .collect();
    if buttons.len() != 1 {
        return Err(RegistrationError::AmbiguousTarget);
    }
    Ok(Some(plan(vec![Action::Tap {
        target: Target::FramePoint {
            frame: observed.frame,
            point: buttons[0].bounds.center(),
        },
    }])))
}

pub fn confirm_password<'a>(
    observed: &Observation,
    inputs: &'a InputValues,
) -> Result<PasswordEvidence<'a>, RegistrationError> {
    if !valid_page(observed) || !matching_identity(observed, inputs) {
        return Err(RegistrationError::WrongPage);
    }
    let expected = inputs
        .get("password")
        .filter(|text| !text.is_empty())
        .ok_or(RegistrationError::PasswordNotConfirmed)?;
    let email = observed
        .elements
        .iter()
        .find(|element| inputs.get("email") == Some(element.text.trim()))
        .ok_or(RegistrationError::WrongPage)?;
    let marketing = observed
        .elements
        .iter()
        .find(|element| element.text.starts_with("Je souhaite recevoir"))
        .ok_or(RegistrationError::WrongPage)?;
    let masked = observed
        .elements
        .iter()
        .filter(|element| {
            let text = element.text.trim();
            element.bounds.y > email.bounds.y + email.bounds.height
                && element.bounds.y + element.bounds.height < marketing.bounds.y
                && text.chars().count() == expected.encode_utf16().count()
                && text
                    .chars()
                    .all(|character| matches!(character, '•' | '●' | '·' | '*'))
        })
        .count();
    let save_prompt = decline_password_save(observed)?.is_some();
    if masked != 1 && !save_prompt {
        return Err(RegistrationError::PasswordNotConfirmed);
    }
    Ok(PasswordEvidence {
        frame: observed.frame,
        inputs,
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Checkboxes {
    pub marketing: bool,
    pub terms: bool,
    marketing_point: Point,
    terms_point: Point,
    frame: Frame,
}

fn checkbox(
    observed: &Observation,
    image: &RgbImage,
    first: &str,
    last: &str,
) -> Result<(bool, Point), RegistrationError> {
    let starts: Vec<_> = observed
        .elements
        .iter()
        .filter(|e| e.text.trim().starts_with(first))
        .collect();
    let ends: Vec<_> = observed
        .elements
        .iter()
        .filter(|e| e.text.trim().ends_with(last))
        .collect();
    if starts.len() != 1 || ends.len() != 1 {
        return Err(RegistrationError::AmbiguousTarget);
    }
    let first = starts[0].bounds;
    let last = ends[0].bounds;
    let point = Point {
        x: f64::from(observed.frame.width) * 0.072,
        y: (first.y + last.y + last.height) / 2.,
    };
    if last.y < first.y || first.x <= point.x || !observed.frame.contains(point) {
        return Err(RegistrationError::WrongPage);
    }
    let radius = (f64::from(observed.frame.width) * 0.04).round() as i64;
    let (mut total, mut teal, mut gray, mut white) = (0usize, 0usize, 0usize, 0usize);
    for y in point.y as i64 - radius..=point.y as i64 + radius {
        for x in point.x as i64 - radius..=point.x as i64 + radius {
            if x < 0 || y < 0 || x >= i64::from(image.width()) || y >= i64::from(image.height()) {
                return Err(RegistrationError::UnknownCheckbox);
            }
            let [r, g, b] = image.get_pixel(x as u32, y as u32).0.map(i32::from);
            total += 1;
            teal += usize::from(r < 90 && g > 70 && b > 70 && g - r > 35 && b - r > 35);
            gray += usize::from(
                r.max(g).max(b) - r.min(g).min(b) < 40
                    && r.min(g).min(b) < 190
                    && r.max(g).max(b) > 60,
            );
            white += usize::from(r > 225 && g > 225 && b > 225);
        }
    }
    let ratio = |count: usize| count as f64 / total as f64;
    if ratio(teal) > 0.2 {
        Ok((true, point))
    } else if ratio(teal) < 0.02 && ratio(white) > 0.75 && ratio(gray) > 0.015 {
        Ok((false, point))
    } else {
        Err(RegistrationError::UnknownCheckbox)
    }
}

/// Deliberately limited to the observed French portrait layout. Blank/ambiguous
/// image areas are not accepted as unchecked boxes. Never blindly toggle a box.
pub fn inspect_checkboxes(
    observed: &Observation,
    image: &RgbImage,
) -> Result<Checkboxes, RegistrationError> {
    if !valid_page(observed) || image.dimensions() != (observed.frame.width, observed.frame.height)
    {
        return Err(RegistrationError::WrongPage);
    }
    let (marketing, marketing_point) =
        checkbox(observed, image, "Je souhaite recevoir", "de Vinted.")?;
    let (terms, terms_point) = checkbox(observed, image, "En t'inscrivant", "18 ans.")?;
    Ok(Checkboxes {
        marketing,
        terms,
        marketing_point,
        terms_point,
        frame: observed.frame,
    })
}

/// Read current OCR and screenshot through the same existing control session.
/// The caller owns input, so no other controller may move the page between reads.
pub async fn inspect_phone(
    client: &PhoneHttpClient,
) -> Result<(Observation, Checkboxes), RegistrationError> {
    let mut device = ZxTouchDevice::new(client.clone());
    let observed = device.observe().await?;
    if !valid_page(&observed) {
        return Err(RegistrationError::WrongPage);
    }
    let image = tokio::time::timeout(Duration::from_secs(15), async {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "content-type",
            "application/json".parse().expect("static header"),
        );
        let mut response = client
            .request(
                reqwest::Method::POST,
                "/api/device/screenshot",
                headers,
                b"{}".to_vec(),
            )
            .await
            .map_err(|_| DeviceError::Unavailable)?;
        if response.status() != 200 {
            return Err(DeviceError::Rejected);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| DeviceError::InvalidResponse)?
        {
            if bytes.len() + chunk.len() > 8 * 1024 * 1024 {
                return Err(DeviceError::InvalidResponse);
            }
            bytes.extend_from_slice(&chunk);
        }
        let dimensions =
            image::ImageReader::with_format(std::io::Cursor::new(&bytes), image::ImageFormat::Jpeg)
                .into_dimensions()
                .map_err(|_| DeviceError::InvalidResponse)?;
        if dimensions != (observed.frame.width, observed.frame.height)
            || u64::from(dimensions.0) * u64::from(dimensions.1) > 16 * 1024 * 1024
        {
            return Err(DeviceError::InvalidResponse);
        }
        image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg)
            .map(|image| image.into_rgb8())
            .map_err(|_| DeviceError::InvalidResponse)
    })
    .await
    .map_err(|_| DeviceError::Unavailable)??;
    let checked = inspect_checkboxes(&observed, &image)?;
    Ok((observed, checked))
}

pub fn consent_changes(current: Checkboxes, preferences: Preferences) -> Option<Workflow> {
    let mut actions = Vec::new();
    for (actual, wanted, point) in [
        (
            current.marketing,
            preferences.marketing,
            current.marketing_point,
        ),
        (current.terms, preferences.accept_terms, current.terms_point),
    ] {
        if actual != wanted {
            actions.push(Action::Tap {
                target: Target::FramePoint {
                    frame: current.frame,
                    point,
                },
            });
        }
    }
    (!actions.is_empty()).then(|| plan(actions))
}

/// Requires a fresh visual check after consent changes and before submission.
/// The result is one submit tap; it never declares that an account was created.
pub fn submit(
    observed: &Observation,
    checked: Checkboxes,
    preferences: Preferences,
    inputs: &InputValues,
    password: &PasswordEvidence<'_>,
) -> Result<Workflow, RegistrationError> {
    if !preferences.accept_terms || !checked.terms {
        return Err(RegistrationError::TermsNotAccepted);
    }
    if !valid_page(observed)
        || observed.frame != password.frame
        || !std::ptr::eq(inputs, password.inputs)
        || observed.frame != checked.frame
        || checked.marketing != preferences.marketing
        || has(observed, Field::Password.label())
        || ![Field::Username, Field::Email].iter().all(|field| {
            inputs
                .get(field.key())
                .is_some_and(|expected| has(observed, expected))
        })
    {
        return Err(RegistrationError::WrongPage);
    }
    let buttons: Vec<_> = observed
        .elements
        .iter()
        .filter(|e| {
            e.text.trim() == "Continuer"
                && e.bounds.center().y > f64::from(observed.frame.height) * 0.4
        })
        .collect();
    if buttons.len() != 1 {
        return Err(RegistrationError::AmbiguousTarget);
    }
    Ok(plan(vec![Action::Tap {
        target: Target::FramePoint {
            frame: observed.frame,
            point: buttons[0].bounds.center(),
        },
    }]))
}

#[cfg(test)]
mod tests;
