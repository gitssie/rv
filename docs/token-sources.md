# Token sources

Token source is internal metadata, derived from the associated phone backup and
explicit phone/RV handover. The shop panel does not display a source label or
provide a device/source selector.
First import of phone state assigns that phone as the token source and uses the
imported access token directly. Later backup imports preserve the chosen source.
Handover changes refresh responsibility; an existing non-rejected,
unexpired token remains usable. A refresh icon next to the source can explicitly
read live credentials even while the cached token is valid. There is no additional shared/phone
business mode: any operation may use its current non-rejected access token.

RV source owns refresh-token rotation. A newly rejected token wakes its already-enabled
keepalive task once; manually stopped keepalive remains stopped. Phone source only reads the current mobile
access token through the existing device-owned authenticated HTTP/2 controller.
It never invokes the Vinted refresh endpoint or changes the stored refresh token.
Phone disconnection does not invalidate a cached access token. Metadata expiry
triggers a phone read; HTTP 401 establishes rejection of that exact token.

Automatic reads check for missing/rejected/expired credentials on source selection,
reconnect and subsequent polling. A valid imported token does not cause a phone
read just because the controller reconnects. Task initialization schedules RV
keepalive at its next normal hourly cron occurrence, never one second after import.
The controller checks every 30 seconds while credentials need updating, separately
from business Tasks. Automatic reads have a 10-second cooldown and concurrent
requests share a result. No App launch, backup operation or phone-side token refresh
is performed. Opening/using the phone App remains what lets that App refresh itself.

The phone reads Shadow's current environment IDFV and its scoped Keychain token.
RV only saves the response for the configured source device and matching
Bundle/IDFV, rechecking this association in the credential transaction. A stale
phone read cannot overwrite a source change. A stale RV refresh response likewise
cannot replace newer credentials or phone ownership of refresh responsibility.
Live access updates do not rewrite saved state archives.

Enabled business tasks wait with `error_code=access_token`, preserving cron and
enabled flags. They make no further requests using a rejected token. New credentials
wake only these token waiters through the existing Credentials EventBus; tasks
manually stopped or completed one-shot tasks stay stopped/completed. Receiving the
same rejected token does not clear its rejection. A different token permits a new
attempt; the business server remains the authority on validity.

Schema 14 adds `app_token_sources`. No row means RV source; a row binds the phone
source to a saved device. If that device is deleted, the source remains phone with
no available device rather than silently granting refresh responsibility to RV.
Existing state handoff/archive workflows remain separate from HTTP admission.

Explicit phone-to-RV backup handover rereads that environments own latest phone
state (also when inactive), adopts its full token pair and metadata, and commits
RV source together with credentials. Failure preserves the old source and
credentials. Catalogue refresh never performs this handover. Enabled token
waiters may resume; stopped tasks remain stopped.
