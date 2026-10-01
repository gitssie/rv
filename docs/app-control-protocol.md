# TrollVNC App control v1

RV's session toolbar launches an installed iOS App by clicking its real icon.
Right-click provides **重启 App**, **关闭 App**, and **移出快捷栏**. The `+` panel searches
the installed App list by name or Bundle ID, and adds/removes favorites. Drag a
shortcut before another icon to reorder it; drop on `+` to move it to the end.
Favorites (up to 32) belong to a saved connection and survive restart. Unsaved
connections keep changes for the current session. A dot marks the last reported
foreground App, refreshed every two seconds. This is not a list of background
processes. The installed list must load successfully before launch controls enable.

Both RV and TrollVNC need these changes. The App extension is independent of
Tight security and of the File Transfer setting. An older/non-TrollVNC peer can
ignore the pseudo encoding and continue normal RFB input/frame updates. RV never
sends App requests until it receives capabilities. The `+` panel stays minimal when support is unavailable. Client view-only and server view-only both disable mutations; favorites
can still be edited locally. In narrow windows, keyboard shortcuts move into the
terminal-icon popover so App shortcuts and connection controls remain available.

## Negotiation and wire format

After authentication, RV includes pseudo encoding `0xC0A1A990` in SetEncodings.
An implementing server immediately replies with operation 0 / request ID 0.
All frames use a 12-byte header followed by UTF-8 JSON:

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 1 | Message type: client 139, server 140 |
| 1 | 1 | Version: 1 |
| 2 | 1 | Operation |
| 3 | 1 | Status: request 0; reply 0 success / 1 error |
| 4 | 4 | Request ID, big endian; zero for capabilities and passive lock events, nonzero for commands |
| 8 | 4 | JSON byte length, big endian |

Requests are at most 4 KiB, replies at most 1 MiB. Invalid header lengths are
rejected before allocating the body. There is no Tight handshake requirement.

| Operation | Request JSON | Successful reply JSON |
| --- | --- | --- |
| 0 capabilities | server initiated | `{"list":true,"launch":true,"terminate":true,"restart":true,"foreground":true,"icons":true,"control":true}` |
| 1 list | `{}` | `{"apps":[{"bundle_id":"com.example.app","name":"Example","can_launch":true,"can_terminate":true}]}` |
| 2 launch | `{"bundle_id":"com.example.app"}` | `{"bundle_id":"com.example.app"}` |
| 3 terminate | same | same |
| 4 restart | same | same |
| 5 foreground | `{}` | `{"bundle_id":"com.example.app"}` or `{"bundle_id":null}` |
| 6 icon | `{"bundle_id":"com.example.app"}` | `{"png":"base64..."}` or `{"png":null}` |

Errors use status 1 and `{"error":"reason"}`. Capability flags reflect runtime
API availability; `control` additionally reflects server view-only. Clients
match IDs and operations, ignore stale responses, and verify mutation target IDs.
Icons are 64×64 PNGs. RV restricts decoded dimensions to 128×128, keeps up to
160 decoded icons, and fetches favorites plus the current 60-App search page.
Explicit refresh retries failed/missing icons; failures do not retry in a loop.

## Execution and limitations

The server serializes App work on a queue, holds a LibVNCServer client reference
for pending work, and locks `sendMutex` when writing each complete response.
Each client can queue at most eight requests. Requests waiting more than eight
seconds expire before execution. RV allows four outstanding requests, preserves
click order among mutations, and times out after 12 seconds without automatic
replay. A client timeout means the operation's result is unknown.

App enumeration/open uses available `LSApplicationWorkspace` / `LSApplicationProxy`
selectors. Hidden services, App Clips and unavailable placeholders are excluded.
Launch success means iOS accepted the open request, not that an unlocked screen
has already displayed it. Lock state, freezing policy, bootstrap permissions and
iOS private API changes can cause failure; the toolbar displays the server error.
Foreground and icon APIs can be unavailable independently. Unconfigured
connections default to Safari, Photos, Settings and App Store, preserving
explicitly saved shortcuts (including an empty list). These Apps, plus optional
WeChat and ProjectX shortcuts, have bundled color fallback icons shown immediately
even before capabilities arrive. App Store uses the local macOS icon as its
fallback; the other assets were recreated from the approved toolbar design.
Device-provided icons take priority. Other missing icons use a graphic
placeholder. UIKit icon rendering uses a bounded main-thread wait; pending
icon work does not indefinitely block server teardown.

Termination resolves the installed App's canonical executable path, enumerates
same-UID processes (also mobile UID 501 when the server runs as root), compares exact executable paths, and rechecks PID start time
before signals. It uses SIGTERM, then SIGKILL if the same process remains, and
rescans the exact target before reporting success. It never uses `killall` or a
name-only match. Safari, Photos, Settings, ProjectX and other ordinary Apps
support terminate/restart. SpringBoard, backboardd, TrollVNC, RootHide and listed
bootstrap management Apps remain protected. Restart requires verified
termination before opening again. System policy may relaunch an App; that is an
error rather than a successful termination.

## Validation

`cargo test --workspace --locked` covers favorites migration/empty preservation,
reorder, request ordering, response target validation, plain RFB App list/icon/open
with concurrent framebuffer updates, and ordinary frame/input against an old
peer. GPUI tests click real shortcut and `+` elements, ensure search input stays
local, reject duplicate/read-only commands, and check icon refresh recovery.
Build TrollVNC with its normal Theos arm64 / RootHide setup. Actual iOS launch,
icon/foreground SPI and process termination require a real device for final
runtime verification; localhost fixtures cannot verify those APIs.
`bash tests/run_app_management_policy_tests.sh` in TrollVNC exercises the
production capability/execution policy, including ordinary Apple Apps,
protected components and mobile process discovery under root sessions.

## Screen lock and unlock

The lock icon opens a local masked setting on first use. Only six ASCII digits
are accepted. VNC login and screen-unlock codes share the client's existing local
credential file, with the unlock code scoped to the connection and host/port.
RV never uses an OS keychain. Right-click the lock icon to replace or clear the setting.

The same toolbar button performs both actions: a closed lock unlocks the phone,
an open lock locks it, and a loading animation blocks repeated clicks while either
operation runs. Progress and success do not add text tips; failures remain visible.
The server sends the current lock state on connection and pushes changes, including
physical locking and unlocking. RV does not poll state while idle.

Capability `unlock` enables operations 7 (screen state), 8 (prepare passcode UI),
and 9 (arm keyboard input, `{"digits":6}`; zero cancels). State replies include
`locked`, `passcode_required`, `input_ready`, `input_empty`, and `reason`; arm
replies include `armed`. Neither request nor reply JSON contains the passcode.

Capability `lock` enables operation 10 (`{}`), which returns the same screen-state
fields. It sends one short power-key press when the screen is on and unlocked,
or the dedicated lock HID event if it is already blanked. Already locked devices
return their state without a power-key toggle. The server waits up to two seconds
for the lock state, and RV polls for confirmation without replaying the lock action.
Client and server are updated together for this protocol.

RV polls every 500 ms, waits for a ready empty passcode field and an arm reply,
then sends six digits using ordinary RFB keyboard down/up events. State requests
time out after two seconds and can retry; the total attempt times out after
12 seconds. The code itself is sent once per click and is never automatically
replayed. Reply correlation prevents a previous attempt from starting new input.
Automatic unlock key events are excluded from TrollVNC key logging.

The server reads lock state through SpringBoardServices and display state through
the existing system notification state. When the display is off it sends a power
key press, waits for wake-up, and swipes from the physical screen bottom to open
the passcode page. AXRuntime confirms the ready empty passcode UI before input. It does not inject a SpringBoard helper.
The native wake/swipe/AX/keyboard flow was verified on iPhone X with iOS 16.7.10.
Other iOS versions still require device verification.
`RV_MOCK_UNLOCK=1 cargo run -p rv-session --example mock_server -- 127.0.0.1:5999`
provides a local six-key fixture and suppresses key logging.

### Passive lock-state event

Server-only operation 11 uses request ID 0 and success status 0, with the minimal
JSON payload `{"locked":true}` or `{"locked":false}`. Capabilities remain operation
0 / ID 0; ordinary command replies retain their nonzero request ID. RV processes
the event without matching or consuming a pending command.

TrollVNC listens to `com.apple.springboard.lockstate` through Darwin notifications,
then reads the actual lock state via SpringBoardServices once for all negotiated
clients. Each client receives an initial state and subsequent changes only.
The notification is also used in [Appium WebDriverAgent's implementation](https://github.com/appium/WebDriverAgent/blob/master/WebDriverAgentLib/Categories/XCUIDevice%2BFBHelpers.m).
This path does not enumerate AX elements, poll periodically, or include passcodes.
Read-only connections receive the same passive state.

RV updates the toolbar's observed lock state; an active operation keeps its loader.
Password readiness, six keyboard digits, confirmation polling, and timeouts remain
in RV's existing request flow. The passive event cannot arm input or complete an
attempt by itself. The simulation sends operation 11 at negotiation and on lock/
unlock changes. Physical iOS notification delivery still needs device validation.
