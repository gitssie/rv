# meow-rs Session proxy feasibility spike

This crate is isolated from RV business routing and the desktop application. It
uses real `meow-common` / `meow-proxy` crates pinned to upstream commit
`59da792d91f10598ce1d9f0afcb99be59150c22c`, with default protocol features disabled.
It does not read RV databases, phone backups, or residential-proxy credentials.

The experimental loopback HTTP CONNECT entrance has one port. Basic-auth user
identifies a session key; its password is the gateway credential supplied by the
host. The in-memory key/value map contains immutable meow chain objects:

```
Session key -> SOCKS5 front -> HTTP CONNECT or SOCKS5 exit -> destination
```

Upstream usernames may be identical while their session passwords differ. Every
chain contains both adapters; a failed front or exit returns an error. Replacing
or removing a key changes future connections while active leases retain their
old chain. `reap_idle` only removes unreferenced idle entries. Session keys and
route credentials are not persisted. The caller owns session allocation,
password-template expansion, exit-IP verification and housekeeping intervals.

Run from the RV repository:

```sh
cargo test -p rv-proxy-spike
cargo check -p rv-business -p rv-session
```

Tests use only loopback fixtures. They check HTTP and SOCKS5 exits, authentication
isolation, delegated domain names, replacement/removal, active-versus-idle
cleanup, mandatory-front failures, upstream rejection, buffered tunnel bytes,
and TCP half-close. These four tests pass on the local macOS host.

## Result: plain proxy patch resolves the local native TLS collision

Unmodified upstream pulls BoringSSL even with default protocol features disabled.
Linking it with RV's OpenSSL produced duplicate symbols and a SIGSEGV in the
HTTPS probe. `patches/optional-proxy-tls.patch` makes proxy TLS optional while
preserving upstream's default TLS support. The patch changes four meow-proxy
files: the feature manifest, HTTP/SOCKS5 TLS wrapping, and HTTPS health probes.
Without the `tls` feature, requesting proxy TLS returns an explicit error;
meow's own HTTPS health probes likewise fail rather than downgrading to HTTP.
Application HTTPS remains handled by the caller, such as RV's rustls client.

With the patch and default features disabled, all five RV spike tests passed
on the local macOS host, including a real HTTPS request through the proxy chain
to an OpenSSL server in the same process. The selected dependency tree contains
no BoringSSL. Explicitly enabling the new meow `tls` feature also passed a type
check, but must not be enabled alongside RV's OpenSSL until the conflict is
resolved separately. This does not verify the complete desktop application.

Reproduce against a local meow checkout (the script leaves RV's workspace and
lockfile untouched; build artifacts use RV's target directory):

```sh
git clone https://github.com/meow-rs/meow-rs.git /path/to/meow-rs
git -C /path/to/meow-rs checkout -b spike/optional-proxy-tls 59da792d91f10598ce1d9f0afcb99be59150c22c
git -C /path/to/meow-rs apply /path/to/rv/crates/rv-proxy-spike/patches/optional-proxy-tls.patch
python3 crates/rv-proxy-spike/verify_plain_proxy.py /path/to/meow-rs
```

The verifier checks the proxy changes against the pinned upstream revision;
the patch may be committed on the local branch. It
runs the crate in a temporary workspace with explicit local Cargo patches, not
by changing the production dependency pin. No production route depends on this
crate. Running the `openssl-compat` feature directly against unpatched upstream
still reproduces the original crash and may produce a core dump.

This is a feasibility harness, not a production replacement for sing-box. Its
entrance accepts CONNECT only: plain HTTP forwarding, a SOCKS5/mixed entrance,
UDP, business-route integration and multi-platform packaging are outside this
spike. It has not verified the provider's Session lifetime or distinct public
IPs. The unmodified upstream dependency still requires BoringSSL; the local
plain-proxy patch avoids it. Do not run the RV desktop build
or app packager to execute this spike.
