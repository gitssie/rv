use std::{future::Future, time::Duration};

use tokio::{sync::watch, time::Instant};

use crate::{
    Action, Device, DeviceError, InputValues, MAX_INPUT_UTF16, Observation, Point, Target, Workflow,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Request {
    Running,
    Paused,
    Cancelled,
}

/// Pause takes effect between steps; cancel also interrupts the active step.
/// Cancellation is latched: resume can never undo it. Use a new control per run.
#[derive(Clone)]
pub struct RunControl(watch::Sender<Request>);
impl Default for RunControl {
    fn default() -> Self {
        Self(watch::channel(Request::Running).0)
    }
}
impl RunControl {
    pub fn pause(&self) {
        self.set(Request::Paused);
    }
    pub fn resume(&self) {
        self.set(Request::Running);
    }
    pub fn cancel(&self) {
        self.set(Request::Cancelled);
    }
    fn set(&self, request: Request) {
        self.0.send_if_modified(|current| {
            if *current == Request::Cancelled || *current == request {
                return false;
            }
            *current = request;
            true
        });
    }
    /// Wait for cancellation when coordinating business steps outside the runner.
    pub async fn cancelled(&self) {
        let mut receiver = self.0.subscribe();
        loop {
            if *receiver.borrow_and_update() == Request::Cancelled {
                return;
            }
            if receiver.changed().await.is_err() {
                return;
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunState {
    Idle,
    Running,
    Paused,
    Succeeded,
    Cancelled,
    Failed,
    OutcomeUnknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureReason {
    InvalidWorkflow,
    InvalidOptions,
    MissingInput,
    InvalidInput,
    InvalidObservation,
    UnexpectedApp,
    TargetNotFound,
    AmbiguousTarget,
    DeviceRejected,
    DeviceUnavailable,
    InvalidResponse,
    StepTimedOut,
    RunTimedOut,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Failure {
    pub reason: FailureReason,
    pub mutation_may_have_executed: bool,
}
impl Failure {
    fn known(reason: FailureReason) -> Self {
        Self {
            reason,
            mutation_may_have_executed: false,
        }
    }
    fn device(error: DeviceError, mutation: bool) -> Self {
        Self {
            reason: match error {
                DeviceError::Rejected => FailureReason::DeviceRejected,
                DeviceError::Unavailable | DeviceError::OutcomeUnknown => {
                    FailureReason::DeviceUnavailable
                }
                DeviceError::InvalidResponse => FailureReason::InvalidResponse,
            },
            mutation_may_have_executed: mutation && error != DeviceError::Rejected,
        }
    }
}

/// Safe to display/log: includes no input values, OCR strings, or server errors.
/// Succeeded means the declared steps completed, not that a business transaction
/// succeeded. Include an explicit WaitForText step to verify an observable result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Progress {
    pub state: RunState,
    /// Zero-based index, None before preflight and for empty/invalid workflows.
    pub step: Option<usize>,
    pub completed_steps: usize,
    pub total_steps: usize,
    pub failure: Option<Failure>,
}
impl Progress {
    fn initial(total_steps: usize) -> Self {
        Self {
            state: RunState::Idle,
            step: None,
            completed_steps: 0,
            total_steps,
            failure: None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RunOptions {
    /// Includes pauses; prevents a forgotten paused task retaining input forever.
    pub timeout: Duration,
    pub poll_interval: Duration,
}
impl Default for RunOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(120),
            poll_interval: Duration::from_millis(250),
        }
    }
}

/// Owns a device and runs one workflow at a time. The application must also
/// exclude manual input and other runners for this phone for the full run.
pub struct Runner<D> {
    device: D,
    options: RunOptions,
    progress: watch::Sender<Progress>,
}
impl<D: Device> Runner<D> {
    pub fn new(device: D) -> Self {
        Self::with_options(device, RunOptions::default())
    }
    pub fn with_options(device: D, options: RunOptions) -> Self {
        Self {
            device,
            options,
            progress: watch::channel(Progress::initial(0)).0,
        }
    }
    /// Latest progress is retained, including terminal status; slow readers may
    /// skip intermediate updates and never block device execution.
    pub fn subscribe(&self) -> watch::Receiver<Progress> {
        self.progress.subscribe()
    }

    pub async fn run(
        &mut self,
        workflow: &Workflow,
        inputs: &InputValues,
        control: &RunControl,
    ) -> Progress {
        let mut progress = Progress::initial(workflow.steps.len());
        self.progress.send_replace(progress.clone());
        let result = self.execute(workflow, inputs, control, &mut progress).await;
        match result {
            Ok(()) => progress.state = RunState::Succeeded,
            Err(failure) => {
                progress.state = if failure.mutation_may_have_executed {
                    RunState::OutcomeUnknown
                } else if failure.reason == FailureReason::Cancelled {
                    RunState::Cancelled
                } else {
                    RunState::Failed
                };
                progress.failure = Some(failure);
            }
        }
        self.progress.send_replace(progress.clone());
        progress
    }

    async fn execute(
        &mut self,
        workflow: &Workflow,
        inputs: &InputValues,
        control: &RunControl,
        progress: &mut Progress,
    ) -> Result<(), Failure> {
        if !workflow.is_valid() {
            return Err(Failure::known(FailureReason::InvalidWorkflow));
        }
        if !(Duration::from_millis(1)..=Duration::from_secs(3600)).contains(&self.options.timeout)
            || !(Duration::from_millis(50)..=Duration::from_secs(10))
                .contains(&self.options.poll_interval)
        {
            return Err(Failure::known(FailureReason::InvalidOptions));
        }
        // Validate all input references before the first possible mutation.
        for step in &workflow.steps {
            if let Action::InputText { input } | Action::WaitForInput { input, .. } = &step.action {
                let value = inputs
                    .get(input)
                    .ok_or(Failure::known(FailureReason::MissingInput))?;
                if value.is_empty()
                    || value.encode_utf16().count() > MAX_INPUT_UTF16
                    || value.chars().any(char::is_control)
                {
                    return Err(Failure::known(FailureReason::InvalidInput));
                }
            }
        }
        let run_deadline = Instant::now() + self.options.timeout;
        for (index, step) in workflow.steps.iter().enumerate() {
            progress.step = Some(index);
            boundary(control, run_deadline, &self.progress, progress).await?;
            let deadline = Deadline::new(run_deadline, Duration::from_millis(step.timeout_ms));
            match &step.action {
                Action::WaitForText { selector, present } => loop {
                    let observed = bounded(self.device.observe(), control, deadline, false).await?;
                    check_observation(&observed, &workflow.bundle_id)?;
                    let found = observed
                        .elements
                        .iter()
                        .any(|element| selector.matches(element, observed.frame));
                    if found == *present {
                        break;
                    }
                    bounded(
                        async {
                            tokio::time::sleep(self.options.poll_interval).await;
                            Ok(())
                        },
                        control,
                        deadline,
                        false,
                    )
                    .await?;
                },
                Action::WaitForInput {
                    input,
                    case_sensitive,
                    region,
                } => {
                    let expected = inputs
                        .get(input)
                        .ok_or(Failure::known(FailureReason::MissingInput))?;
                    let selector = crate::TextSelector {
                        text: String::new(),
                        mode: crate::TextMatch::Exact,
                        case_sensitive: *case_sensitive,
                        region: *region,
                    };
                    loop {
                        let observed =
                            bounded(self.device.observe(), control, deadline, false).await?;
                        check_observation(&observed, &workflow.bundle_id)?;
                        if observed.elements.iter().any(|element| {
                            selector.matches_value(expected, element, observed.frame)
                        }) {
                            break;
                        }
                        bounded(
                            async {
                                tokio::time::sleep(self.options.poll_interval).await;
                                Ok(())
                            },
                            control,
                            deadline,
                            false,
                        )
                        .await?;
                    }
                }
                Action::Tap { target } => {
                    let observed = bounded(self.device.observe(), control, deadline, false).await?;
                    check_observation(&observed, &workflow.bundle_id)?;
                    let point = locate(target, &observed)?;
                    bounded(
                        self.device.tap(observed.frame, point),
                        control,
                        deadline,
                        true,
                    )
                    .await?;
                }
                Action::InputText { input } => {
                    let observed = bounded(self.device.observe(), control, deadline, false).await?;
                    check_observation(&observed, &workflow.bundle_id)?;
                    // Preflight above checked every reference before any action.
                    let text = inputs
                        .get(input)
                        .ok_or(Failure::known(FailureReason::MissingInput))?;
                    bounded(self.device.input_text(text), control, deadline, true).await?;
                }
                Action::Delay { milliseconds } => {
                    bounded(
                        async {
                            tokio::time::sleep(Duration::from_millis(*milliseconds)).await;
                            Ok(())
                        },
                        control,
                        deadline,
                        false,
                    )
                    .await?;
                }
            }
            progress.completed_steps = index + 1;
            self.progress.send_replace(progress.clone());
        }
        Ok(())
    }
}

fn check_observation(observation: &Observation, expected_bundle: &str) -> Result<(), Failure> {
    if !observation.is_valid() {
        return Err(Failure::known(FailureReason::InvalidObservation));
    }
    if observation.bundle_id.as_deref() != Some(expected_bundle) {
        return Err(Failure::known(FailureReason::UnexpectedApp));
    }
    Ok(())
}

fn locate(target: &Target, observation: &Observation) -> Result<Point, Failure> {
    let frame = observation.frame;
    let point = match target {
        Target::FramePoint {
            frame: expected,
            point,
        } => {
            if *expected != frame {
                return Err(Failure::known(FailureReason::TargetNotFound));
            }
            *point
        }
        Target::Screen { point } => Point {
            x: point.x * f64::from(frame.width),
            y: point.y * f64::from(frame.height),
        },
        Target::Text { selector, offset } => {
            let mut matches = observation
                .elements
                .iter()
                .filter(|element| selector.matches(element, frame));
            let first = matches
                .next()
                .ok_or(Failure::known(FailureReason::TargetNotFound))?;
            if matches.next().is_some() {
                return Err(Failure::known(FailureReason::AmbiguousTarget));
            }
            let center = first.bounds.center();
            Point {
                x: center.x + offset.x * f64::from(frame.width),
                y: center.y + offset.y * f64::from(frame.height),
            }
        }
    };
    if !frame.contains(point) {
        return Err(Failure::known(FailureReason::TargetNotFound));
    }
    Ok(point)
}

#[derive(Clone, Copy)]
struct Deadline {
    at: Instant,
    reason: FailureReason,
}
impl Deadline {
    fn new(run: Instant, step_timeout: Duration) -> Self {
        let step = Instant::now() + step_timeout;
        if run <= step {
            Self {
                at: run,
                reason: FailureReason::RunTimedOut,
            }
        } else {
            Self {
                at: step,
                reason: FailureReason::StepTimedOut,
            }
        }
    }
}

async fn bounded<T>(
    future: impl Future<Output = Result<T, DeviceError>>,
    control: &RunControl,
    deadline: Deadline,
    mutation: bool,
) -> Result<T, Failure> {
    // Do not even poll a mutation if cancellation/deadline is already visible.
    if *control.0.borrow() == Request::Cancelled {
        return Err(Failure::known(FailureReason::Cancelled));
    }
    if Instant::now() >= deadline.at {
        return Err(Failure::known(deadline.reason));
    }
    tokio::select! {
        biased;
        _ = control.cancelled() => Err(Failure { reason: FailureReason::Cancelled, mutation_may_have_executed: mutation }),
        _ = tokio::time::sleep_until(deadline.at) => Err(Failure { reason: deadline.reason, mutation_may_have_executed: mutation }),
        result = future => result.map_err(|error| Failure::device(error, mutation)),
    }
}

async fn boundary(
    control: &RunControl,
    deadline: Instant,
    sender: &watch::Sender<Progress>,
    progress: &mut Progress,
) -> Result<(), Failure> {
    let mut receiver = control.0.subscribe();
    loop {
        let request = *receiver.borrow_and_update();
        if request == Request::Cancelled {
            return Err(Failure::known(FailureReason::Cancelled));
        }
        if Instant::now() >= deadline {
            return Err(Failure::known(FailureReason::RunTimedOut));
        }
        progress.state = if request == Request::Paused {
            RunState::Paused
        } else {
            RunState::Running
        };
        sender.send_replace(progress.clone());
        if request == Request::Running {
            return Ok(());
        }
        tokio::select! {
            biased;
            _ = receiver.changed() => {},
            _ = tokio::time::sleep_until(deadline) => return Err(Failure::known(FailureReason::RunTimedOut)),
        }
    }
}
