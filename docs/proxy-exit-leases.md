# Proxy exit leases

`rv-business::proxy::ProxyManager` prepares a stable exit for a logical task.
Settings → 代理 uses **换 IP 方式** to select HTTP URL or 密码 Session.
Both implementations use the same `ProxyManager::acquire(..., IpPolicy::Rotate)`
entry and the same exit observation flow. The existing encrypted SQLite proxy
record stores the method, URL/session marker and probe URL; saving never rotates.
Old records default to HTTP URL.

HTTP URL sends an HTTP GET provider control request. 密码 Session replaces the
alphanumeric value immediately following the configured marker (default
`_session-`) with a new lowercase random value of the same length, preserving all
other password parameters. For example, `_country-fr_lifetime-5_session-a7ozznml`
changes only `a7ozznml`. The new password is committed atomically before creating
a new client/full front→exit sing-box route. It remains stored if observation
fails, so future requests do not revert to the old provider session. No Vinted
request header or access-token Session is changed. Changing the password is not
proof that the exit changed; both methods observe the resulting exit.

The **检测 IP** button tests the current form without saving. It uses the configured
SOCKS5h proxy and probe URL, respects proxy occupancy, and formats JSON `ip`, country, region and city fields for display. Unrecognized
responses are displayed as text. Manual detection does not validate IP format, public/private
ranges, address family, or API success fields; HTTP error bodies are displayed too. Detection never rotates or writes IP history. New proxies
request location fields by default; opening a proxy with the former built-in
IP-only URL expands its visible form URL to include those fields (saved only on
Save). Other custom URLs remain unchanged.
Location fields follow the [provider documentation](https://ipwhois.io/documentation).

```rust,ignore
use rv_business::proxy::{ProxyManager, IpPolicy};

let owner = uuid::Uuid::new_v4().to_string(); // one owner per logical task
let manager = ProxyManager::new(paths.clone());
let lease = manager.acquire_bound(&app_id, &owner, IpPolicy::NeverSeen).await?;
let inner = lease.reenter(IpPolicy::Reuse).await?;
// Perform the full operation/response body while both leases remain in scope.
let response = inner.client().get(business_url).send().await?;
let body = response.bytes().await?;
drop(inner);
drop(lease);
```

For a task without an App ID, `acquire(proxy_id, owner, policy)` accepts an
explicit existing proxy ID. The caller supplies routing context; this module
does not choose a proxy or change phone networking. Registration currently has
no automatic call to this API. Its eventual coordinator must supply the intended
proxy context and retain the lease for the whole registration, including phone
operations if that phone uses the same exit.

## Policy and ownership

- `Reuse`: query and record the current IPv4; do not rotate.
- `Rotate`: request a change and confirm a different, stable IPv4.
- `NeverSeen`: additionally require an atomic first insertion in the shared IP
  history. Repeated IPs cause another confirmed rotation, up to the configured
  limit.

Managed leases are exclusive even for an otherwise shared proxy. They use the
same process-local gate as business sync, chat and image requests. Ordinary
shared/exclusive behavior is retained outside managed scopes. This gate is not
a cross-process/distributed lock; the SQLite uniqueness constraint still
protects the history across connections/processes using the same database.

Owner identity is a logical task identifier, not an IDFV. Reuse it for nested
calls; independent tasks must use different owners. `acquire_bound` takes an
explicit owner, resolves the existing IDFV→proxy binding and never substitutes
another proxy. Business and managed owners have separate enum namespaces so a
normal business request cannot enter a managed rotation in progress. Accidentally
using a currently held business owner fails rather than waiting for itself.

The first managed acquire fixes preparation for the scope. Parallel/nested
acquires join the same result. Reentry never rotates again. A stricter request
is accepted only when the prepared result already satisfies it; otherwise it
fails. A nested helper must use `lease.reenter()`/`lease.client()`, rather than
opening an ordinary route that would wait for the outer managed lease.

The last release unlocks the exit. A preparation error, timeout, dropped future
or panic also releases its count. If initialization is interrupted, remaining
reentrant waiters receive `PreparationInterrupted`; they cannot replay it.
After the entire scope ends, a new explicit task may prepare again.

## Network sequence

Waiting for the gate does not consume the preparation deadline. After acquiring
it, proxy configuration is reloaded from SQLite. IP queries and work use a
SOCKS5h client with remote DNS, no direct fallback, no HTTP retry and no reuse of
idle connections from before rotation. The provider change link uses a separate
direct, no-redirect/no-retry client.

The sequence records the previous IPv4, invokes the selected rotation implementation once, and polls
through the selected proxy until two consecutive observations agree on a
different IPv4. If the control reply is lost, polling reconciles the effect;
another change is not blindly sent. A confirmed but historically seen exit can
advance to another attempt. Defaults: at most 3 changes, 20 seconds to settle
each change, 1 second between observations, 75 seconds overall preparation.
`AcquireOptions` supplies bounded overrides.

IP query responses may be plain IPv4 text or JSON with an `ip` field. An explicit
`success:false`, malformed response, IPv6, or non-public IPv4 is rejected. The
special-purpose validation follows the
[IANA IPv4 registry](https://www.iana.org/assignments/iana-ipv4-special-registry/).
HTTP body sizes, network deadlines and retries are bounded; errors exclude URLs,
credentials and response bodies.

## Global history

Schema version 6 adds `proxy_ip_history`, keyed only by canonical IPv4 text.
`INSERT ... ON CONFLICT(ip) DO NOTHING` performs check-and-claim in one SQLite
statement. The first observation records time, proxy and task owner. All proxy
IDs share it; competing exits cannot both claim one fresh IP. Every observed IP
is retained even if preparation or work later fails. Releasing a lease never
deletes history; deleting a proxy only clears its history foreign key.

Validation covers real local SOCKS5/HTTP peers, duplicate-IP rotation, one-time
reentry preparation, legacy gate interaction, interrupted/unknown mutations,
bounded exhaustion, cross-exit atomic claims, encrypted URL persistence and
SQLite upgrades. Tests never call a real change-IP endpoint.
