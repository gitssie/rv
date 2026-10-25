use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};

use tokio::sync::Notify;

use crate::*;

const APP: &str = "local.fixture.app";
fn frame() -> Frame {
    Frame {
        width: 1000,
        height: 2000,
        orientation: 0,
    }
}
fn observation(texts: &[&str]) -> Observation {
    Observation {
        frame: frame(),
        bundle_id: Some(APP.into()),
        elements: texts
            .iter()
            .enumerate()
            .map(|(i, text)| TextElement {
                text: (*text).into(),
                bounds: Rect {
                    x: 100.,
                    y: 100. + i as f64 * 100.,
                    width: 200.,
                    height: 40.,
                },
            })
            .collect(),
    }
}
fn selector(text: &str) -> TextSelector {
    TextSelector {
        text: text.into(),
        mode: TextMatch::Exact,
        case_sensitive: false,
        region: None,
    }
}
fn workflow(actions: Vec<Action>) -> Workflow {
    Workflow {
        version: WORKFLOW_VERSION,
        bundle_id: APP.into(),
        steps: actions
            .into_iter()
            .enumerate()
            .map(|(i, action)| Step {
                id: format!("step_{i}"),
                timeout_ms: 5000,
                action,
            })
            .collect(),
    }
}
fn tap() -> Action {
    Action::Tap {
        target: Target::Text {
            selector: selector("Email"),
            offset: Point { x: 0., y: 0.05 },
        },
    }
}
fn input() -> Action {
    Action::InputText {
        input: "email".into(),
    }
}
fn values() -> InputValues {
    let mut inputs = InputValues::default();
    inputs.insert("email", "test@example.invalid".into());
    inputs
}

#[derive(Clone, Debug, PartialEq)]
enum Call {
    Observe,
    Tap(Frame, Point),
    Input,
}
struct FakeDevice {
    calls: Arc<Mutex<Vec<Call>>>,
    observations: VecDeque<Observation>,
    pending_read: bool,
    pending_mutation: bool,
    error: Option<DeviceError>,
    started: Arc<Notify>,
}
impl FakeDevice {
    fn new(observations: Vec<Observation>) -> Self {
        Self {
            calls: Arc::default(),
            observations: observations.into(),
            pending_read: false,
            pending_mutation: false,
            error: None,
            started: Arc::default(),
        }
    }
    async fn mutation(&self) -> Result<(), DeviceError> {
        self.started.notify_one();
        if self.pending_mutation {
            std::future::pending::<()>().await;
        }
        self.error.map_or(Ok(()), Err)
    }
}
impl Device for FakeDevice {
    fn observe(&mut self) -> DeviceFuture<'_, Observation> {
        Box::pin(async move {
            self.calls.lock().unwrap().push(Call::Observe);
            if self.pending_read {
                self.started.notify_one();
                std::future::pending::<()>().await;
            }
            self.observations
                .pop_front()
                .ok_or(DeviceError::Unavailable)
        })
    }
    fn tap(&mut self, frame: Frame, point: Point) -> DeviceFuture<'_, ()> {
        Box::pin(async move {
            self.calls.lock().unwrap().push(Call::Tap(frame, point));
            self.mutation().await
        })
    }
    fn input_text<'a>(&'a mut self, text: &'a str) -> DeviceFuture<'a, ()> {
        Box::pin(async move {
            assert_eq!(text, "test@example.invalid");
            self.calls.lock().unwrap().push(Call::Input);
            self.mutation().await
        })
    }
}

#[tokio::test(start_paused = true)]
async fn observes_again_for_each_action_and_waits_for_actual_result() {
    let device = FakeDevice::new(vec![
        observation(&[]),
        observation(&["Email"]),
        observation(&["Email"]),
        observation(&["Email"]),
        observation(&["Done"]),
    ]);
    let calls = device.calls.clone();
    let mut runner = Runner::new(device);
    let flow = workflow(vec![
        Action::WaitForText {
            selector: selector("Email"),
            present: true,
        },
        tap(),
        input(),
        Action::WaitForText {
            selector: selector("Done"),
            present: true,
        },
    ]);
    let progress = runner.run(&flow, &values(), &RunControl::default()).await;
    assert_eq!(progress.state, RunState::Succeeded);
    assert_eq!(progress.completed_steps, 4);
    assert_eq!(
        *calls.lock().unwrap(),
        vec![
            Call::Observe,
            Call::Observe,
            Call::Observe,
            Call::Tap(frame(), Point { x: 200., y: 220. }),
            Call::Observe,
            Call::Input,
            Call::Observe
        ]
    );
    assert_eq!(*runner.subscribe().borrow(), progress);
}

#[tokio::test]
async fn all_inputs_are_checked_before_first_mutation() {
    let device = FakeDevice::new(vec![observation(&["Email"])]);
    let calls = device.calls.clone();
    let result = Runner::new(device)
        .run(
            &workflow(vec![tap(), input()]),
            &InputValues::default(),
            &RunControl::default(),
        )
        .await;
    assert_eq!(result.failure.unwrap().reason, FailureReason::MissingInput);
    assert!(calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn refuses_ambiguous_or_outside_targets_and_foreground_changes() {
    for (observed, action, reason) in [
        (
            observation(&["Email", "EMAIL"]),
            tap(),
            FailureReason::AmbiguousTarget,
        ),
        (observation(&[]), tap(), FailureReason::TargetNotFound),
        (
            observation(&["Email"]),
            Action::Tap {
                target: Target::Text {
                    selector: selector("Email"),
                    offset: Point { x: -1., y: 0. },
                },
            },
            FailureReason::TargetNotFound,
        ),
        (
            Observation {
                bundle_id: Some("other.app".into()),
                ..observation(&["Email"])
            },
            tap(),
            FailureReason::UnexpectedApp,
        ),
        (
            Observation {
                frame: Frame {
                    orientation: 4,
                    ..frame()
                },
                ..observation(&["Email"])
            },
            tap(),
            FailureReason::InvalidObservation,
        ),
    ] {
        let device = FakeDevice::new(vec![observed]);
        let calls = device.calls.clone();
        let result = Runner::new(device)
            .run(&workflow(vec![action]), &values(), &RunControl::default())
            .await;
        assert_eq!(result.failure.unwrap().reason, reason);
        assert_eq!(*calls.lock().unwrap(), vec![Call::Observe]);
    }
}

#[tokio::test]
async fn screen_points_use_observed_native_landscape_frame() {
    let landscape = Frame {
        width: 2000,
        height: 1000,
        orientation: 1,
    };
    let device = FakeDevice::new(vec![Observation {
        frame: landscape,
        ..observation(&[])
    }]);
    let calls = device.calls.clone();
    let result = Runner::new(device)
        .run(
            &workflow(vec![Action::Tap {
                target: Target::Screen {
                    point: Point { x: 0.25, y: 0.75 },
                },
            }]),
            &values(),
            &RunControl::default(),
        )
        .await;
    assert_eq!(result.state, RunState::Succeeded);
    assert_eq!(
        calls.lock().unwrap()[1],
        Call::Tap(landscape, Point { x: 500., y: 750. })
    );
}

#[tokio::test(start_paused = true)]
async fn read_timeout_is_known_but_mutation_timeout_is_unknown_without_replay() {
    for mutation in [false, true] {
        let mut device = FakeDevice::new(vec![observation(&["Email"])]);
        device.pending_read = !mutation;
        device.pending_mutation = mutation;
        let calls = device.calls.clone();
        let result = Runner::new(device)
            .run(
                &workflow(vec![input(), tap()]),
                &values(),
                &RunControl::default(),
            )
            .await;
        assert_eq!(
            result.state,
            if mutation {
                RunState::OutcomeUnknown
            } else {
                RunState::Failed
            }
        );
        assert_eq!(result.failure.unwrap().reason, FailureReason::StepTimedOut);
        assert_eq!(result.completed_steps, 0);
        assert_eq!(
            *calls.lock().unwrap(),
            if mutation {
                vec![Call::Observe, Call::Input]
            } else {
                vec![Call::Observe]
            }
        );
    }
}

#[tokio::test]
async fn cancellation_distinguishes_active_reads_and_mutations() {
    for mutation in [false, true] {
        let mut device = FakeDevice::new(vec![observation(&["Email"])]);
        device.pending_read = !mutation;
        device.pending_mutation = mutation;
        let started = device.started.clone();
        let control = RunControl::default();
        let flow = workflow(vec![tap(), input()]);
        let inputs = values();
        let mut runner = Runner::new(device);
        let (result, ()) = tokio::join!(runner.run(&flow, &inputs, &control), async {
            started.notified().await;
            control.cancel();
            control.resume(); // Cancellation cannot be undone.
        });
        assert_eq!(
            result.state,
            if mutation {
                RunState::OutcomeUnknown
            } else {
                RunState::Cancelled
            }
        );
        assert_eq!(result.failure.unwrap().reason, FailureReason::Cancelled);
        assert_eq!(result.completed_steps, 0);
    }
}

#[tokio::test]
async fn pre_cancelled_run_never_contacts_device() {
    let device = FakeDevice::new(vec![observation(&["Email"])]);
    let calls = device.calls.clone();
    let control = RunControl::default();
    control.cancel();
    let result = Runner::new(device)
        .run(&workflow(vec![tap()]), &values(), &control)
        .await;
    assert_eq!(result.state, RunState::Cancelled);
    assert!(calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn pause_at_boundary_can_resume_without_losing_progress() {
    let device = FakeDevice::new(vec![observation(&["Email"])]);
    let calls = device.calls.clone();
    let mut runner = Runner::new(device);
    let mut receiver = runner.subscribe();
    let control = RunControl::default();
    control.pause();
    let flow = workflow(vec![tap()]);
    let inputs = values();
    let (result, ()) = tokio::join!(runner.run(&flow, &inputs, &control), async {
        loop {
            receiver.changed().await.unwrap();
            if receiver.borrow_and_update().state == RunState::Paused {
                break;
            }
        }
        assert!(calls.lock().unwrap().is_empty());
        control.resume();
    });
    assert_eq!(result.state, RunState::Succeeded);
}

#[tokio::test(start_paused = true)]
async fn global_deadline_includes_pause_and_has_correct_reason() {
    for pause in [false, true] {
        let mut device = FakeDevice::new(vec![observation(&["Email"])]);
        device.pending_read = true;
        let control = RunControl::default();
        if pause {
            control.pause();
        }
        let options = RunOptions {
            timeout: Duration::from_secs(1),
            ..RunOptions::default()
        };
        let result = Runner::with_options(device, options)
            .run(&workflow(vec![tap()]), &values(), &control)
            .await;
        assert_eq!(result.state, RunState::Failed);
        assert_eq!(result.failure.unwrap().reason, FailureReason::RunTimedOut);
    }
}

#[tokio::test]
async fn rejected_and_unknown_mutations_stop_later_steps() {
    for error in [
        DeviceError::Rejected,
        DeviceError::OutcomeUnknown,
        DeviceError::InvalidResponse,
    ] {
        let mut device = FakeDevice::new(vec![observation(&["Email"])]);
        device.error = Some(error);
        let calls = device.calls.clone();
        let result = Runner::new(device)
            .run(
                &workflow(vec![tap(), input()]),
                &values(),
                &RunControl::default(),
            )
            .await;
        assert_eq!(
            result.state,
            if error == DeviceError::Rejected {
                RunState::Failed
            } else {
                RunState::OutcomeUnknown
            }
        );
        assert_eq!(calls.lock().unwrap().len(), 2);
        assert_eq!(result.completed_steps, 0);
    }
}

#[test]
fn workflow_validation_and_serialization_exclude_runtime_secrets() {
    let flow = workflow(vec![tap(), input()]);
    assert!(flow.is_valid());
    let encoded = serde_json::to_string(&flow).unwrap();
    assert_eq!(serde_json::from_str::<Workflow>(&encoded).unwrap(), flow);
    assert!(!encoded.contains("test@example.invalid"));
    assert!(!format!("{:?}", values()).contains("test@example.invalid"));
    let mut invalid = flow.clone();
    invalid.version += 1;
    assert!(!invalid.is_valid());
    invalid = flow.clone();
    invalid.steps[1].id = invalid.steps[0].id.clone();
    assert!(!invalid.is_valid());
    invalid = flow;
    invalid.steps[0].action = Action::Tap {
        target: Target::Screen {
            point: Point {
                x: f64::NAN,
                y: 0.5,
            },
        },
    };
    assert!(!invalid.is_valid());
    assert!(
        serde_json::from_str::<Workflow>(
            r#"{"version":1,"bundle_id":"local.app","steps":[],"password":"secret"}"#
        )
        .is_err()
    );
}

#[tokio::test]
async fn prevents_keyboard_control_characters_and_unbounded_input_before_io() {
    for text in ["test\nsubmit".into(), "😀".repeat(65), "".into()] {
        let device = FakeDevice::new(vec![]);
        let calls = device.calls.clone();
        let mut inputs = InputValues::default();
        inputs.insert("email", text);
        let result = Runner::new(device)
            .run(&workflow(vec![input()]), &inputs, &RunControl::default())
            .await;
        assert_eq!(result.failure.unwrap().reason, FailureReason::InvalidInput);
        assert!(calls.lock().unwrap().is_empty());
    }
}

#[test]
fn selector_region_disambiguates_and_matching_modes_are_explicit() {
    let mut selected = selector("email");
    selected.region = Some(Rect {
        x: 0.,
        y: 0.,
        width: 1.,
        height: 0.1,
    });
    let observed = observation(&["Email", "Email"]);
    assert!(selected.matches(&observed.elements[0], frame()));
    assert!(!selected.matches(&observed.elements[1], frame()));
    selected.case_sensitive = true;
    assert!(!selected.matches(&observed.elements[0], frame()));
    selected.text = "Mai".into();
    selected.mode = TextMatch::Contains;
    selected.case_sensitive = false;
    assert!(selected.matches(&observed.elements[0], frame()));
}
