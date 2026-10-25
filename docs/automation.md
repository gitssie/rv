# RV automation foundation

`rv-automation` is a Rust workspace crate compiled through `rv-session`. It needs
no Python interpreter, script service, or additional phone daemon. There is no
registration workflow, toolbar action, automatic submission, or business data
model in this change. Existing VNC/gesture behavior is unchanged.

## Boundaries

- `rv-automation`: versioned workflow definitions, runtime input values, selectors,
  native frame geometry, device trait, sequential runner, control and progress.
- `rv-session::automation::ZxTouchDevice`: the existing authenticated HTTP/2 client
  adapted to foreground/OCR, frame-bound native tap and keyboard input.
- Future business strategies: choose the workflow by Bundle ID, provide values,
  decide postconditions and own persistence through RV's existing SQLite layer.
- Future GPUI integration: own the task, display progress, and acquire the
  phone's exclusive input lease before starting. The runner does not acquire the
  existing VNC/manual-input gate or arbitrate separately constructed runners.

Workflow definitions support serde serialization; this module creates no JSON
configuration files, SQLite schema or second credential store. Runtime values
are separate, non-serializable `InputValues`, with redacted Debug and zeroing of
their owned string copies on drop. The caller must protect its original copies.

## Steps and selectors

Version 1 supports:

| Action | Behavior |
| --- | --- |
| `WaitForText` | Poll fresh OCR until text is present/absent, within the step deadline |
| `Tap` | Resolve one target against fresh OCR/frame metadata, then send one tap |
| `InputText` | Insert a runtime value into the focused field; no implicit clear/submit |
| `Delay` | Cancellable delay, bounded by step/run deadlines |

Text selectors specify exact/contains matching, case sensitivity and an optional
screen-relative search region. A text tap requires exactly one match; ambiguity
or an off-screen offset fails without tapping. A text target's offset is measured
in fractions of the screen, added to the OCR box center. Screen targets also use
fractions in `[0,1)`. All dispatched points use native screenshot pixels, not VNC
canvas coordinates. Frame width, height and orientation travel with each tap so
the phone can reject a changed frame.

Every observation validates geometry and the expected foreground Bundle ID.
The HTTP adapter samples foreground before and after OCR and rejects a detected
app change. These are separate requests, not an atomic guarantee of input focus;
business policies must establish the right field and verify the visible result.
This foundation uses OCR, not an iOS accessibility tree or control identifiers.

Inputs are checked before the first device request. Each text value is limited
to 128 UTF-16 units and excludes control characters. The limit keeps a native
ASCII keyboard command (~50ms/key) below the existing HTTP deadline. Commands
are never automatically chunked or replayed. The phone uses its foreground app
adapter's text insertion when available; the daemon implementation uses hardware
key events for ASCII and clipboard/Command+V for non-ASCII, which may show the
app's normal paste UI. Both native success envelopes are supported. An
acknowledged command alone does not prove
that a field accepted the text, especially for secure inputs.

## Calling from a business module

```rust,ignore
use rv_automation::{InputValues, RunControl, Runner, Workflow};
use rv_session::automation::ZxTouchDevice;

// The owning session first acquires exclusive input for this phone.
// `client` is its existing authenticated PhoneHttpClient.
let mut runner = Runner::new(ZxTouchDevice::new(client));
let mut progress = runner.subscribe();
let control = RunControl::default();
let task_control = control.clone();

// `workflow: Workflow` contains selectors and InputText keys, not input values.
// `inputs: InputValues` comes from the business module's existing data source.
let task = tokio::spawn(async move {
    runner.run(&workflow, &inputs, &task_control).await
});

// The UI can observe progress.changed(), call control.pause()/resume(), or:
control.cancel();
let terminal = task.await?;
// Inspect terminal before releasing the input lease or allowing another task.
```

Pause applies between steps, without replaying the completed step. Cancellation
also interrupts an active step and is latched; a resumed control cannot undo it.
Use a new control for each run. The run deadline includes pauses. Default run
timeout is 120 seconds; each step has its own timeout. Options, workflow version,
IDs, all actions and input references are validated before execution.

The watch-based progress stream retains the latest state and terminal result;
slow consumers can skip intermediate updates without blocking device execution.
Progress carries step indices/counts and structured errors, never OCR strings,
input contents, server responses or credentials. It is not an audit event log.

## Completion and interrupted mutations

`Succeeded` means the declared steps completed. It does not assert registration
success or SDK/server acceptance. Include explicit observable postconditions in
the business workflow. Password fields generally need another success condition
because masked contents cannot be checked through OCR.

Read/delay cancellation is `Cancelled`; known rejection is `Failed`. Cancellation,
timeout, broken transport or malformed acknowledgement during a mutation is
`OutcomeUnknown`: the command may already have changed the phone. No subsequent
step runs and no mutation is resent. Inspect the actual device state before a
new run. Do not use `JoinHandle::abort` as normal cancellation; signal the control
and await completion. The native tap handler releases a dispatched touch even
when its HTTP request is cancelled; the engine never leaves its own begin/end
sequence open. Future drag/gesture actions must preserve that cleanup contract.

## Validation

```sh
cargo test -p rv-automation --locked
cargo test -p rv-session automation:: --locked
cargo check -p rv-app --tests --locked
```

Tests use a deterministic fake device for lifecycle/geometry checks and a real
local HTTP/2 peer for the production ZXTouch adapter, including interrupted
keyboard input without replay. They do not send touches to a live phone or
establish acceptance for a particular registration form.
