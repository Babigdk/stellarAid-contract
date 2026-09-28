# Authorization Matrix

Every state-changing entry point in the campaign, donation, withdrawal and
dispute-arbiter contracts, and the address that is allowed to authorize it.

The matrix is normative: an entry point that is not listed here is either
read-only or permissionless by design, and a listed entry point that does not
enforce its row is a bug.

## Why a documented matrix

Soroban authorization is enforced at the call site, so nothing stops a new
entry point from being added with the wrong check — or with none. This document
makes the intended authorization for every function reviewable on its own, and
`contracts/campaign/src/auth_tests.rs` asserts each enforced row by calling the
function from an address that is *not* authorized.

## Two halves of an authorization check

Both halves are required, and the campaign contract previously enforced only
one of them in several places:

| Half | What it does | Fails when |
|---|---|---|
| **Signature** — `actor.require_auth()` | Proves the named actor signed this invocation | The actor did not sign |
| **Identity** — `ensure_admin(&env, &actor)` | Proves the named actor *is* the address the function is scoped to | The actor signed, but is not the stored admin |

`require_auth()` alone is satisfied by *any* address that signs. A function
whose only check is `require_auth()` is therefore callable by anyone who
nominates themselves. Every admin-only row below requires both halves.

## Matrix

Legend for **Enforced**: *this PR* = enforced by the accompanying commit;
*existing* = already enforced on `main`; *other contract* = enforced by the
contract that owns the function.

### Campaign — user-facing

| Function | Authorized actor | Checks | Enforced |
|---|---|---|---|
| `initialize` | the admin being installed | `admin.require_auth()`, `AlreadyInitialized` guard | existing |
| `create_campaign` | `owner` (the creator) | `owner.require_auth()` | existing |
| `set_campaign_asset_code` | `owner`, and `owner == campaign.owner` | `owner.require_auth()` + creator identity | *this PR* |
| `update_raised` | donation contract (cross-contract) | frozen / paused / `initialized` / campaign open | existing |
| `record_donor` | donation contract (cross-contract) | frozen | *this PR* |
| `finalize_payout` | `campaign.owner` | `owner.require_auth()` + `recipient == campaign.owner` | *this PR* |
| `add_dispute_evidence` | `campaign.owner` **or** a recorded donor | `submitter.require_auth()` + donor/creator identity | *this PR* |

### Campaign — admin

| Function | Authorized actor | Checks | Enforced |
|---|---|---|---|
| `set_admin` | current admin **and** new admin (dual auth) | both `require_auth()` | existing |
| `transfer_admin` | current admin | `current_admin.require_auth()` + admin identity | existing |
| `upgrade` | the admin stored at `initialize` | stored admin's `require_auth()`; the caller is not a parameter | *this PR* |
| `freeze` / `unfreeze` | stored admin | stored admin's `require_auth()` | existing |
| `flag_for_review` / `clear_review_flag` | stored admin | stored admin's `require_auth()` | existing |
| `pause` / `unpause` | stored admin | `require_auth()` + `ensure_admin` | existing |
| `update_campaign_status` | stored admin | `require_auth()` + `ensure_admin` | existing |
| `approve_campaign` / `reject_campaign` / `suspend_campaign` | stored admin | `require_auth()` + `ensure_admin` | existing |
| `archive_campaign` | stored admin | `require_auth()` + `ensure_admin` | existing |
| `set_alert_config` | stored admin | `require_auth()` **+ `ensure_admin`** | *this PR* |
| `report_ok` / `report_error` | stored admin | `require_auth()` **+ `ensure_admin`** | *this PR* |
| `set_feature_flag` | stored admin | `require_auth()` **+ `ensure_admin`** | *this PR* |
| `set_canary_deployment` | stored admin | `require_auth()` **+ `ensure_admin`** | *this PR* |
| `set_rollback_trigger` | stored admin | `require_auth()` **+ `ensure_admin`** | *this PR* |
| `trigger_rollback` | stored admin | `require_auth()` **+ `ensure_admin`** | *this PR* |

The seven health/rollout setters are the rows this PR fixes. They previously
performed `require_auth()` and stopped there, so any account could rewrite the
alerting config, flip a feature flag, re-point the canary deployment, or trigger
a platform-wide rollback by naming itself as `admin`. `trigger_rollback` in
particular sets `Paused` in the shared pause module, so it was a
denial-of-service primitive. The same seven-row gap existed verbatim in
`contracts/withdrawal` and `contracts/donation` and is fixed in both.

### Donation

| Function | Authorized actor | Checks | Enforced |
|---|---|---|---|
| `initialize` | the admin being installed | `admin.require_auth()` | existing |
| `donate` | the donor — **unless** `anonymous` | `donor.require_auth()` when `!anonymous` | existing |
| `refund` | the donation admin or the campaign owner | `caller.require_auth()` + identity check | existing |
| `pause` / `unpause` / `upgrade` | stored admin | `require_auth()` + `ensure_admin` | existing |
| health / rollout setters | stored admin | `require_auth()` **+ `ensure_admin`** | *this PR* |

`donate`'s conditional check is deliberate: an anonymous donation intentionally
takes no donor signature so a donor cannot be linked to a campaign. The price
is that `anonymous: true` accepts any caller's amount, which is why the campaign
side is the only place donation *sizes* are trusted.

### Withdrawal

| Function | Authorized actor | Checks | Enforced |
|---|---|---|---|
| `initialize` | the admin being installed | `admin.require_auth()` | existing |
| `request_withdrawal` | `owner` | `owner.require_auth()` | existing |
| `approve_withdrawal` | stored admin | `require_auth()` + `ensure_admin` | existing |
| `reject_withdrawal` | stored admin | `require_auth()` + `ensure_admin` | existing |
| `pause` / `unpause` / `upgrade` | stored admin | `require_auth()` + `ensure_admin` | existing |
| health / rollout setters | stored admin | `require_auth()` **+ `ensure_admin`** | *this PR* |

### Dispute arbiter

| Function | Authorized actor | Checks | Enforced |
|---|---|---|---|
| `initialize` | permissionless once-initialization guard | `has_admin` gate | existing |
| `open_dispute` | the dispute initiator | `initiator.require_auth()` | existing |
| `resolve_for_client` / `resolve_for_artist` / `partial_resolve` | the arbiter admin, resolved from PlatformConfig | `admin.require_auth()` | existing |
| `auto_resolve` | permissionless once the auto-resolve ledger is due | ledger check only | existing |

`auto_resolve` is intentionally permissionless: anyone may settle a dispute
once it is overdue, and it only ever moves funds in the direction the escrow
already committed to. Making it admin-gated would leave disputes stuck whenever
the admin key is unavailable.

## Rows the issue names but `main` does not have yet

The issue's matrix also lists `request_withdrawal`, `finalize_withdrawal`,
`cancel_withdrawal_request`, `end_campaign`, `cancel_campaign`,
`extend_deadline`, `raise_dispute` and `resolve_dispute` for the campaign
contract. None of these exist on `main`; the campaign lifecycle is landing in
separate branches. Their intended authorization is pinned here so it is decided
before the code is written:

| Function | Authorized actor | Rationale |
|---|---|---|
| `request_withdrawal` | `campaign.owner` (creator) | Only the creator may schedule raised funds |
| `finalize_withdrawal` | `campaign.owner` (creator) | The payout target is the creator |
| `cancel_withdrawal_request` | `campaign.owner` (creator) | Cancelling a creator's own request |
| `end_campaign` | `campaign.owner` **or** admin | An admin must be able to close an abandoned campaign |
| `cancel_campaign` | `campaign.owner` **or** admin | Same, plus refused while `raised > 0` so donor funds cannot be stranded |
| `extend_deadline` | `campaign.owner` (creator) | Pushing a deadline is a creator-side promise to donors |
| `raise_dispute` | `campaign.owner` **or** a recorded donor | "Donor or creator", per the issue |
| `resolve_dispute` | stored admin | Arbitration is not delegated to participants |

## Permissionless entry points

Read-only and intentionally unauthenticated:

* `get_campaign`, `get_campaign_count`, `get_fee_config`,
  `get_campaign_asset_code`, `get_dispute_evidence_count`, `can_cover_payout`
* `get_version`, `get_version_metadata`, `is_version_compatible`
* `health_check`, `get_health_metrics`, `get_sla_targets`, `get_alert_config`,
  `detect_anomaly`, `is_feature_enabled`, `get_rollout_state`, `should_rollback`,
  `route_to_canary`
* `is_frozen`, `is_under_review`, `get_review_reason`

`bump_campaign_ttl` is the one borderline case: it is state-changing but only
extends a TTL, so it is left unauthenticated. It cannot create, destroy or
transfer value; the worst a caller can do is pay for a longer retention window
on a campaign they do not own.

## Adding an entry point

Before adding a state-changing function, add a row here and a test in
`contracts/campaign/src/auth_tests.rs` that calls it from a non-authorized
address and asserts the failure. A function with no row and no test will not
pass review.
