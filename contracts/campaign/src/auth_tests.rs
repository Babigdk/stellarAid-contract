//! Authorization tests for the campaign contract (issue #763).
//!
//! One test per row of `docs/AUTHORIZATION_MATRIX.md` that this change
//! enforces, each calling the function from an address that is **not**
//! authorized and asserting that it is rejected.
//!
//! # The two halves of a check
//!
//! `require_auth()` proves the named actor signed. `ensure_admin()` proves the
//! named actor *is* the address the function is scoped to. A test that only
//! omits the signature proves nothing about the identity half — which is exactly
//! the gap these tests were written to close, so every admin-only test below
//! runs under `env.mock_all_auths()`: the caller's own signature is always
//! available, and the **identity check alone** has to reject the call.

use crate::{CampaignContract, CampaignContractClient};
use soroban_sdk::{
    testutils::Address as _, testutils::Events as _, Address, BytesN, Env, String, Symbol,
};

struct Fixture {
    env: Env,
    client: CampaignContractClient<'static>,
    admin: Address,
    owner: Address,
    stranger: Address,
    campaign_id: u64,
}

fn setup() -> Fixture {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, CampaignContract);
    let client = CampaignContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let owner = Address::generate(&env);
    let stranger = Address::generate(&env);

    client.initialize(&admin);
    let campaign_id = client.create_campaign(&owner, &10_000_i128, &2_000_u64, &500, &None);

    Fixture {
        env,
        client,
        admin,
        owner,
        stranger,
        campaign_id,
    }
}

/// Asserts that `f` is rejected.
fn assert_rejected(f: impl FnOnce()) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    assert!(result.is_err(), "an unauthorized call must be rejected");
}

// ── Campaign: user-facing entry points ─────────────────────────────────────────

#[test]
fn test_set_campaign_asset_code_rejects_a_non_creator() {
    let f = setup();
    let code = String::from_str(&f.env, "USDC");
    assert_rejected(|| {
        // The stranger signs for themselves: only the creator identity check
        // can reject this.
        f.client
            .set_campaign_asset_code(&f.stranger, &f.campaign_id, &code);
    });
    assert_eq!(f.client.get_campaign_asset_code(&f.campaign_id), None);

    // ...and the creator is accepted.
    f.client
        .set_campaign_asset_code(&f.owner, &f.campaign_id, &code);
    assert_eq!(f.client.get_campaign_asset_code(&f.campaign_id), Some(code));
}

#[test]
fn test_finalize_payout_rejects_a_recipient_that_is_not_the_creator() {
    let f = setup();
    let token_address = Address::generate(&f.env);
    assert_rejected(|| {
        // The creator authorizes, but names a stranger as the payout target:
        // funds may only ever be drawn to the creator.
        f.client
            .finalize_payout(&f.campaign_id, &1, &token_address, &f.stranger);
    });
    // The debited `raised` must be untouched.
    assert_eq!(f.client.get_campaign(&f.campaign_id).unwrap().raised, 0);
}

#[test]
fn test_add_dispute_evidence_rejects_a_stranger() {
    let f = setup();
    let hash = BytesN::from_array(&f.env, &[1u8; 32]);
    assert_rejected(|| {
        f.client
            .add_dispute_evidence(&f.campaign_id, &f.stranger, &hash);
    });
    assert_eq!(f.client.get_dispute_evidence_count(&f.campaign_id), 0);

    // The creator is a party to the dispute and is accepted.
    f.client
        .add_dispute_evidence(&f.campaign_id, &f.owner, &hash);
    assert_eq!(f.client.get_dispute_evidence_count(&f.campaign_id), 1);
}

#[test]
fn test_add_dispute_evidence_rejects_an_unrecorded_donor_claim() {
    // A *recorded* donor is accepted; an address that merely claims to be one is
    // not. `record_donor` is what separates the two.
    let f = setup();
    let hash = BytesN::from_array(&f.env, &[2u8; 32]);
    f.client.record_donor(&f.campaign_id, &f.stranger);
    f.client
        .add_dispute_evidence(&f.campaign_id, &f.stranger, &hash);
    assert_eq!(f.client.get_dispute_evidence_count(&f.campaign_id), 1);

    let impostor = Address::generate(&f.env);
    assert_rejected(|| {
        f.client
            .add_dispute_evidence(&f.campaign_id, &impostor, &hash);
    });
    assert_eq!(f.client.get_dispute_evidence_count(&f.campaign_id), 1);
}

// ── Campaign: the health / rollout identity gap ────────────────────────────────
//
// Every one of these ran under `mock_all_auths`, so the caller's own signature
// was always available. Before the fix, `require_auth()` was the only check, and
// all of them succeeded for an arbitrary address.

#[test]
fn test_set_alert_config_rejects_a_non_admin() {
    let f = setup();
    let config = f.client.get_alert_config();
    assert_rejected(|| f.client.set_alert_config(&f.stranger, &config));
    assert_eq!(f.client.get_alert_config(), config);

    // The admin is still accepted, so the failure above is the identity check
    // and not a blanket breakage.
    f.client.set_alert_config(&f.admin, &config);
}

#[test]
fn test_report_ok_rejects_a_non_admin() {
    let f = setup();
    assert_rejected(|| f.client.report_ok(&f.stranger));
    assert_eq!(f.client.get_health_metrics().ok_count, 0);

    f.client.report_ok(&f.admin);
    assert_eq!(f.client.get_health_metrics().ok_count, 1);
}

#[test]
fn test_report_error_rejects_a_non_admin() {
    let f = setup();
    assert_rejected(|| f.client.report_error(&f.stranger));
    assert_eq!(f.client.get_health_metrics().ok_count, 0);
}

#[test]
fn test_set_feature_flag_rejects_a_non_admin() {
    let f = setup();
    let flag = Symbol::new(&f.env, "beta");
    assert_rejected(|| f.client.set_feature_flag(&f.stranger, &flag, &true));
    assert!(!f.client.is_feature_enabled(&flag));

    f.client.set_feature_flag(&f.admin, &flag, &true);
    assert!(f.client.is_feature_enabled(&flag));
}

#[test]
fn test_set_canary_deployment_rejects_a_non_admin() {
    let f = setup();
    let canary = Address::generate(&f.env);
    let stable = Address::generate(&f.env);
    assert_rejected(|| {
        f.client
            .set_canary_deployment(&f.stranger, &canary, &stable, &1_000);
    });

    f.client
        .set_canary_deployment(&f.admin, &canary, &stable, &1_000);
    assert_eq!(f.client.get_rollout_state().canary_bps, 1_000);
}

#[test]
fn test_set_rollback_trigger_rejects_a_non_admin() {
    let f = setup();
    let before = f.client.get_rollout_state().rollback_error_bps;
    assert_rejected(|| f.client.set_rollback_trigger(&f.stranger, &900));
    assert_eq!(f.client.get_rollout_state().rollback_error_bps, before);

    f.client.set_rollback_trigger(&f.admin, &900);
    assert_eq!(f.client.get_rollout_state().rollback_error_bps, 900);
}

#[test]
fn test_trigger_rollback_rejects_a_non_admin() {
    // `trigger_rollback` sets the shared pause flag, so an unauthorized caller
    // reaching it is a denial-of-service primitive, not a cosmetic gap. The
    // rejected call is also rolled back, so the contract must stay usable.
    let f = setup();
    assert_rejected(|| f.client.trigger_rollback(&f.stranger));
    f.client.create_campaign(&f.owner, &1_000_i128, &2_000_u64, &500, &None);
}

// ── Campaign: upgrade gate ─────────────────────────────────────────────────────

#[test]
fn test_upgrade_rejects_a_signature_that_is_not_the_stored_admin() {
    use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};
    use soroban_sdk::IntoVal;

    let env = Env::default();
    let contract_id = env.register_contract(None, CampaignContract);
    let client = CampaignContractClient::new(&env, &contract_id);
    let admin1 = Address::generate(&env);
    let admin2 = Address::generate(&env);
    let hash = BytesN::from_array(&env, &[9u8; 32]);

    env.mock_all_auths();
    client.initialize(&admin1);
    // Rotating the admin rotates the upgrade key with it.
    client.set_admin(&admin2);

    // Only the *old* admin signs. Because the upgrade gate reads the admin from
    // storage instead of taking it as a parameter, admin1's signature must not
    // be enough.
    env.mock_auths(&[MockAuth {
        address: &admin1,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "upgrade",
            args: (hash.clone(),).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    let before = env.events().all().len();
    assert_rejected(|| client.upgrade(&hash));
    // Authorization is checked before any deployer work, so nothing is emitted.
    assert_eq!(env.events().all().len(), before);
}

#[test]
fn test_upgrade_publishes_nothing_before_authorization_is_satisfied() {
    // The success path ends in `update_current_contract_wasm`, a deployer-only
    // host call that a unit test cannot satisfy, so `contract_upgraded` cannot
    // be observed here. What *is* observable — and what this test pins — is
    // that the event is published only after the stored admin has authorized.
    let f = setup();
    let hash = BytesN::from_array(&f.env, &[4u8; 32]);
    let before = f.env.events().all().len();
    assert_rejected(|| f.client.upgrade(&hash));
    assert_eq!(f.env.events().all().len(), before);
}

// ── Regression sweep ───────────────────────────────────────────────────────────

#[test]
fn test_non_admin_cannot_reach_any_health_or_rollout_setter() {
    // Table-shaped sweep so a setter added later cannot skip the identity check
    // without one of these assertions failing.
    let f = setup();
    let config = f.client.get_alert_config();
    let flag = Symbol::new(&f.env, "beta");
    let canary = Address::generate(&f.env);
    let stable = Address::generate(&f.env);

    assert_rejected(|| f.client.set_alert_config(&f.stranger, &config));
    assert_rejected(|| f.client.report_ok(&f.stranger));
    assert_rejected(|| f.client.report_error(&f.stranger));
    assert_rejected(|| f.client.set_feature_flag(&f.stranger, &flag, &true));
    assert_rejected(|| {
        f.client
            .set_canary_deployment(&f.stranger, &canary, &stable, &10)
    });
    assert_rejected(|| f.client.set_rollback_trigger(&f.stranger, &1));
    assert_rejected(|| f.client.trigger_rollback(&f.stranger));

    // The admin still can, so the failures above are the identity check.
    f.client.report_ok(&f.admin);
}
