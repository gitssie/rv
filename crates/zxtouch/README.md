# ZXTouch Rust client

Standalone HTTP/2 client for the current TrollVNC native control API. It has no RV,
GPUI, database, VNC or Shadow build dependency. Shadow remains the phone-side
source of truth for App-state behavior.

```rust,no_run
use zxtouch::{Api, PhoneHttpClient};
use serde_json::json;
use std::time::Duration;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let api = Api::new(PhoneHttpClient::new("127.0.0.1", 6000, String::new())?);
api.control().connect("password").await?;
let catalog = api.app_state().list(&json!({"bundle_id":"lt.manodrabuziai.fr"})).await?;
let catalog = api.app_state().wait(catalog, Duration::from_secs(30)).await?;
# Ok(())
# }
```

Namespaces: control, apps, device, scripts, apple, app_state, karing_native.
Named methods accept native JSON payloads and preserve fields in responses.
The `ROUTES` inventory describes the complete native API plus raw Karing proxy.
`raw` preserves HTTP status, headers and response bytes/stream. `stream` and
`Ndjson::next` incrementally read bounded events without collecting the stream.
Callers stop on the protocol's verified terminal event; EOF alone is not success.

Apple and App-state share operation validation and polling. `invoke` returns
validated operation records, including HTTP 400 terminal Apple errors. `wait`
checks completion, returns verification challenges, rejects failed/unknown/partial
outcomes and retains IDs on polling failure or timeout. Challenge codes are six
ASCII digits, preserving leading zeros; no automatic challenge submission occurs.

App-state claim/release/ownership are ordinary asynchronous operations. Saved
backup reads/writes carry `requester` when acting as RV; omission remains phone.
This client does not automatically claim, release, restore, delete or retry writes.
`upload` validates chunk bounds and its exact received/complete acknowledgement;
RV's handoff layer retains state-first,
manifest-last ordering and SHA/size/readback verification.

HTTP/2 is used with no proxy, redirects or request retry. Token state is shared in
memory; touch controllers can isolate their transport connection while sharing
authentication. Device toast/vibrate/alert/foreground-pid names intentionally
match current wire routes; consult native task schemas rather than guessing from
route names. Screenshot returns a raw response for caller content-type/status
validation. Raw Karing supports arbitrary HTTP methods and query strings.

Check native coverage with `python3 crates/zxtouch/tools/check_api_coverage.py`
(or pass `--server` for another TrollVNC checkout).

Run `cargo test -p zxtouch` from the RV workspace, or `cargo test --manifest-path
crates/zxtouch/Cargo.toml`.
