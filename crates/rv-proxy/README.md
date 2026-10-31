# RV embedded proxy transport

`rv-proxy` supplies immutable meow HTTP CONNECT / SOCKS5 chains and a single
loopback SOCKS5 / CONNECT gateway. The business engine owns its dedicated Tokio
runtime and authenticates each route with a random key. Existing connections
retain their original chain when a route is removed. No failed hop falls back
to a direct connection. UDP and TLS-wrapped proxy endpoints are not enabled.
Destination HTTPS is handled by the caller (RV uses rustls).

Only `meow-common` and `meow-proxy` are direct upstream dependencies; their
low-level DNS, trie and transport dependencies are pulled transitively. Neither
the meow application nor its listener/config/API engines are dependencies.

The two meow crates come directly from `https://github.com/gitssie/meow-rs`,
pinned to commit `317fa9df484264820368ffca559ed8fcc7188765` on
`feature/optional-proxy-tls`. Default meow protocol features are disabled. The
fork makes proxy TLS optional so this transport does not link BoringSSL beside
RV's OpenSSL. There are no local dependency overrides or vendored meow copies.

Shared password-session proxy templates produce independent in-memory Sessions
per real App IDFV. Business and image clients hold the same Session lease.
Unused client caches expire after 120 seconds; unleased password mappings expire
after 120 seconds and are checked every five seconds. Generated passwords do not
replace the encrypted SQLite template. Exclusive and explicit managed IP rotation
retain the existing gate, confirmation, and history rules.

Run from the RV repository:

```sh
cargo test -p rv-proxy --features openssl-compat
cargo test -p rv-business
cargo check -p rv-app
```

The tests use only loopback fixtures, including HTTPS with RV's OpenSSL and
rustls in the same process, both gateway entrances, and both exit protocols.
They do not establish the provider's public-IP uniqueness or Session lifetime.
No desktop build or packaging is needed.

An explicitly authorized live Google connectivity probe uses an existing saved
front/exit chain without rotating Sessions or changing configuration:

```sh
cargo test -p rv-business live_google_through_configured_chain -- --ignored --nocapture
```

Set `RV_LIVE_PROXY_ID` to select a saved exit; otherwise the probe picks the first
chain, preferring a password-session configuration. It never prints credentials
or response bodies. The live check returned HTTP/2 301 from `google.com` and
HTTP/2 200 at the Google redirect destination on the local test host. This is
connectivity evidence for that selected route, not a public-IP uniqueness test.
