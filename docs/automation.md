# RV automation foundation

`rv-automation` is a Rust workspace crate compiled through `rv-session`. It needs
no Python interpreter, script service, or additional phone daemon. The session
function panel now has sibling Apple / App 环境 / 功能 tabs. 功能 contains the
compact Vinted registration operation, backed by this runner.

## Boundaries

- `rv-automation`: versioned workflow definitions, runtime input values, selectors,
  native frame geometry, device trait, sequential runner, control and progress.
- `rv-session::automation::ZxTouchDevice`: the existing authenticated HTTP/2 client
  adapted to foreground/OCR, frame-bound native tap and keyboard input.
- `automation::vinted`: the observed French portrait registration policy, field
  reconciliation, consent appearance and positive password evidence.
- `functions::registration`: one RV-local task, existing authenticated client,
  sanitized phase updates, cancellation and an overall 180-second deadline.
- `registration_ui`: one four-row textarea, preferences, execute/stop and state.
  It blocks manual input in its owning window while running. The saved-device
  controller rejects overlapping App operations and other function commands.
  This does not lock the physical phone or external VNC clients; the user must
  avoid interacting with the phone during the task. Generic runners constructed
  separately still require their caller to arrange exclusive input.

Workflow definitions support serde serialization; this module creates no JSON
configuration files, SQLite schema or second credential store. Runtime values
are separate, non-serializable `InputValues`, with redacted Debug and zeroing of
their owned string copies on drop. The caller must protect its original copies.

## Steps and selectors

Version 1 supports:

| Action | Behavior |
| --- | --- |
| `WaitForText` | Poll fresh OCR until text is present/absent, within the step deadline |
| `WaitForInput` | Compare OCR against a runtime value without embedding it in the workflow |
| `Tap` | Resolve one target against fresh OCR/frame metadata, then send one tap |
| `InputText` | Insert a runtime value into the focused field; no implicit clear/submit |
| `Delay` | Cancellable delay, bounded by step/run deadlines |

Text selectors specify exact/contains matching, case sensitivity and an optional
screen-relative search region. A text tap requires exactly one match; ambiguity
or an off-screen offset fails without tapping. A text target's offset is measured
in fractions of the screen, added to the OCR box center. Screen targets also use
fractions in `[0,1)`. All dispatched points use native screenshot pixels, not VNC
canvas coordinates. Frame width, height and orientation travel with each tap so
the phone can reject a changed frame. `FramePoint` also rejects a changed frame
before dispatch, useful for points resolved from an earlier observation.

Every observation validates geometry and the expected foreground Bundle ID.
The HTTP adapter samples foreground before and after OCR and rejects a detected
app change. These are separate requests, not an atomic guarantee of input focus;
business policies must establish the right field and verify the visible result.
This foundation uses OCR, not an iOS accessibility tree or control identifiers.

Inputs are checked before the first device request. Each text value is limited
to 128 UTF-16 units and excludes control characters. Input requires the phone's
`keyboard_literal: 1` capability and sends keyboard
action 8: one serialized clipboard-set + native Command+V. This preserves
literal text across keyboard layouts, including AZERTY. It does not change the
system's software-keyboard settings. There is no VNC, legacy TCP or physical-key
fallback. Commands are never chunked or automatically replayed. Both native
success envelopes are supported. ACK alone does not establish accepted input;
the business policy reconciles the field and recognizes only the expected
Vinted/trollvncserver paste permission for its pending insertion.

## Registration panel

The existing 360-wide tool surface is reused; the registration editor has one
four-row textarea, not separate form fields. Enter one account:

```text
name@example.test username password
https://example.test/verification
```

The first line also accepts `email----username----password`. The second line is
optional and validated as an HTTP(S) URL. The observed French email challenge now uses this URL to fetch a six-digit code,
checks the destination email and empty code field, inputs once, reconciles the
visible code and submits once. The observed phone-number gate stops with an
explicit pending message; it is not complete-registration evidence.
No new persistence or credential file is created. Textarea drafts remain in
memory when changing tabs or returning to the menu. Marketing defaults off;
terms default checked per the user's explicit preference, and remain editable.

Open the French Vinted welcome/provider page or an empty registration form on
the phone before executing. The coordinator opens the email form as needed,
requires the empty-field placeholders, fills and reconciles username/email,
checks positive nonempty password evidence, applies the selected marketing and
terms choices, rereads their appearance and sends at most one submit tap.
A missing placeholder by itself is not password evidence. An owned iOS password
save sheet is declined using Plus tard; no credential is saved to keychain.

The final status says registration was submitted and asks the user to complete
verification/check the result on the phone. It never reports account creation
from a tap ACK. A partial, ambiguous, cancelled or disconnected run stops without
replaying. Inspect the phone before another execution. Closing the owning
window/disconnecting cancels; tab changes and hiding the panel preserve the task.

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

## Shared post-paste touch policy (2026-10-28)

Manual RV paste and registration now share `rv-session::paste_touch::PasteTouch`:
150 ms after the paste ACK, hold at the last field tap for 900–1100 ms, release,
wait 80 ms, then hold at the same point for 100–120 ms and release. One bounded
normal-distribution offset (at most two native pixels per axis) is sampled for
both holds. The manual canvas still uses VNC; automation uses authenticated
HTTP/2 only. Timing starts from actual dispatch/ACK, not queued enqueue times.

Automation requires `device_tap_hold=1` in addition to literal keyboard support
before sending field contents. The new TrollVNC 4.0-15 source exposes optional
`hold_ms` on `/api/device/tap`; the native handler polls stream/connection liveness
and always releases a dispatched finger. Cancellation never queues another hold
or replays the clipboard command. Frame and foreground checks guard each hold.
Source/package version alone is not proof of runtime capability: the historical
4.0-14 package rejected keyboard8 during this device session.

The 2026-10-28 live run accepted the supplied email code and reached the phone
number gate, where the user explicitly chose to stop. That run used direct
HTTP touch on the observed number pad, not the newly shared post-paste policy;
it must not be described as end-to-end verification of this code change.
