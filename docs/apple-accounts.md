# Apple ID registry and first-login password rotation

`rv-core` migration 7 creates `apple_accounts`. Apple ID passwords are plaintext by explicit product requirement. Existing encrypted VNC/unlock storage is unchanged.

- `apple_id` is normalized and case-insensitive. `original_password` preserves the first imported password; `password` is the last confirmed current password.
- `pending_password` is generated as exactly eight ASCII alphanumeric characters, with uppercase, lowercase and digits. It is committed before requesting an iOS mutation.
- `verification_code_url`, optional nonunique `device_id`, JSON `params`, note and timestamps are independent fields. Device association has no foreign key or lock.
- `init_status` records pending/changing_password/ready/unknown. `account_status` separately records unknown/active/locked/disabled; generic failures do not imply an Apple lock.

The managed Store-login worker saves/imports the record, uses its stored current password and code URL, signs in, persists the candidate, invokes native `changePassword`, commits only an explicit `password_changed=true` result, then signs in with the new password. Both normal and account-targeted invocation use the same wrapper. A confirmed password change is saved even when another step reports failure. An unconfirmed change retains both passwords and the native operation receipt for result queries.

## Current integration boundary (2026-10-26)

Core storage tests and HTTP/2 mock integration tests pass; these do not establish real-device password-change acceptance. RV checks authorize capability `apple_password_change=1` before invoking its internal `changePassword` operation. The native `changePassword` endpoint is now implemented and passed an isolated production-adapter HTTP/2 device test. The installed phone daemon remains the older version and does not advertise this capability until updated; the isolated test is not a deployed-daemon or live RV GUI acceptance claim.

The existing phone unlock password is loaded through `StorePaths.load_unlock_code` for the original saved endpoint; there is no new unlock credential store. TrollVNC exposes internal `TVAppleValidateLocalPasscode` using the existing ManagedConfiguration/CDP implementation.

### Real-device password change (2026-10-26)

A signed standalone native probe successfully changed the current test account on iPhone9,2 / iOS 15.8.8. It uses the full native `AAUIGrandSlamRemoteUIPresenter` / `RemoteUIController` flow with an offscreen navigation controller; no Settings coordinate input is used. The actual local-passcode challenge was validated with the saved device unlock password before continuing the native PIN action.

The local-passcode branch returns only `newPassword.password` and `newPassword.confirmedPassword`; the trusted-device branch previously returned an additional current-password row. Requiring all three rows incorrectly stopped the probe before submission. The native save URL is `#signoutDevices`, a named alert, **not a JavaScript URL or the final POST**. The probe chose the server-advertised keep-other-devices button and continued through native `alertView:pressedButton:completion:`. Apple returned HTTP 200 from `/appleid/settings/account/manage/security/password/put` and the security-settings model. The probe's string-only `passwordUpdated` success check still did not recognize an explicit success flag; do not call HTTP 200 alone proof of success.

Independent acceptance used the installed ZXTouch HTTP/2 service at the phone Wi-Fi address, port 6000: complete logout and fresh all-domains-signed-out verification, then `signInAppStore` with the pre-saved candidate succeeded (`authenticated=true`, ~10.3 seconds). A separate fresh account query confirmed selected/authenticated target media. Only after this verification was the real database current password advanced and the pending candidate cleared; the original password remains preserved. The phone is now signed into App Store with the new password; iCloud was not activated.

The standalone host harness added the approved table idempotently while preserving the running older RV client's global `user_version=5`; it did not claim migration 7 was applied. Future normal migration 7 can adopt the existing table. The real registry now has initialization `ready`, account `active`, an eight-character current password, no pending candidate and a password-change timestamp.

This initial check proved the standalone native mutation plus subsequent production HTTP/2 login. The next section records the subsequent native endpoint implementation and acceptance.


## Native endpoint and settings browser (2026-10-27)

`TVAppleServices.changeAppleIDPassword` is implemented through NativeBackend and
`TVApplePasswordSession`. Main-thread RemoteUI operations are asynchronous; account
worker authentication/waiting and saved local-passcode validation do not block main.
The flow supports the observed local-passcode and trusted-device continuations,
checks actual two/three-row forms and native validation, then selects only the
server-provided keep-other-devices button in `signoutDevices`. The mutation POST
is never replayed. Native true flags include the observed numeric/string form;
otherwise fresh AuthKit new-credential confirmation may establish success without
switching a different media account. Unsupported/ambiguous responses preserve an
unknown result after submission.

The production-adapter endpoint is `POST /api/apple/changePassword`, with
`apple_id`, `password`, `new_password` and optional `device_passcode`; code URLs
remain host-only. It shares the existing result/challenge API and advertises
`apple_password_change=1` in updated builds. A signed isolated HTTP/2 listener using
the actual adapter, facade, backend and new native session changed the current
account in11.752s and returned `password_changed=true`. The host saved the new
candidate before invoke, stored the operation receipt and committed only after
that result. Independent production Wi-Fi6000 complete logout/fresh signed-out
query/new-password Store login/fresh authenticated-media query then passed in7.331s.
Original password remains preserved; current is eight mixed ASCII characters,
pending is cleared, initialization ready. No other-device logout or iCloud
activation was requested.

The Apple tools panel now exposes **修改密码**. It resolves the fresh current
identity/iCloud account (or authenticated media when no phone identity exists),
loads saved credentials, generates and saves a candidate, then invokes the native
endpoint directly. It does not require a Store login or change a different media
account. Normal first-login rotation reuses the same internal operation. Definite
manual rejection restores an already completed initialization to ready; unknown
retains both credentials and receipt. Internal origin metadata preserves the
manual/first-login behavior when querying an unknown operation.

Settings now includes an **Apple ID** category using `gpui_component::table::DataTable`.
It reads the real SQLite registry, supports literal case-insensitive metadata
search and refresh, and displays current/original/pending passwords, code URL,
weak device association, statuses, parameters, note, errors and timestamps.
Passwords start masked and can be shown with the standard Switch. Search does not
include passwords or treat `%`/`_` as wildcards. Internal workflow metadata is not
shown as user parameters.

Validation: core storage/search/manual-rejection regression tests, managed
first-login/manual/unknown-result HTTP/2 mock tests, native form/flag/confirmation
and adapter-schema tests, app test type-check, UI-style policy and independent
review. The standard rootless TrollVNC package builds. RV was not built/packaged
per project rules, and the phone daemon was not replaced; runtime UI verification
and the full managed GUI-to-phone flow require updating those running builds.
