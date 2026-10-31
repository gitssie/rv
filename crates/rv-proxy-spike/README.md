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

## Result: proxy semantics work; direct RV embedding is blocked

A separate compatibility probe links RV's OpenSSL and meow's BoringSSL into the
same process and attempts a real HTTPS request through the chain. On this host,
the linker reports duplicate OpenSSL/BoringSSL symbols and the test process
terminates with SIGSEGV. Type checking alone does not detect this collision.
Even disabling meow's default protocol features still brings in BoringSSL.

Reproduce only the compatibility failure (expected to fail; may produce a core dump):

```sh
cargo test -p rv-proxy-spike --features openssl-compat openssl_coexistence_probe -- --exact tests::openssl_coexistence_probe
```

The failing probe is opt-in so the independent routing tests remain executable;
it is not waived or counted as passing. No production route depends on this
crate. Safe in-process integration requires resolving native symbol isolation
or eliminating one TLS implementation, then rerunning the probe successfully.
A separate proxy process is another option, but is not validated here. This
spike does not establish HTTPS compatibility inside the existing RV process.

This is a feasibility harness, not a production replacement for sing-box. Its
entrance accepts CONNECT only: plain HTTP forwarding, a SOCKS5/mixed entrance,
UDP, business-route integration and multi-platform packaging are outside this
spike. It has not verified the provider's Session lifetime or distinct public
IPs. Building meow's transport currently requires the BoringSSL C/C++ toolchain
even with optional outbound protocols disabled. Do not run the RV desktop build
or app packager to execute this spike.
