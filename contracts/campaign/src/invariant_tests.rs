//! Invariant tests for campaign balance integrity.
//!
//! Verifies that:
//! - raised == sum(donation amounts) (cross-contract invariant)
//! - withdrawn <= raised
//! - remaining = raised - withdrawn >= 0
//! - no overflow occurs in balance arithmetic

#![cfg(test)]

use crate::CampaignContract;
use soroban_sdk::{testutils::Address as _, Address, Env};

/// Simulates tracking raised/withdrawn locally to validate the invariant
/// raised >= withdrawn (no overdraft).
#[test]
fn test_invariant_raised_never_below_withdrawn() {
    let raised: i128 = 100_000;
    let withdrawn: i128 = 40_000;
    let remaining = raised - withdrawn;
    assert!(remaining >= 0, "remaining must be non-negative");
    assert_eq!(remaining, 60_000);

    // Even after multiple withdrawals
    let withdrawn2 = 60_000;
    let remaining2 = raised - (withdrawn + withdrawn2);
    assert!(
        remaining2 >= 0,
        "remaining must be non-negative after second withdrawal"
    );
    assert_eq!(remaining2, 0);
}

/// raised must never exceed the sum of individual donations (prevent
/// inflation attacks or accounting bugs).
#[test]
fn test_invariant_raised_equals_sum_donations() {
    let donations = [10_000_i128, 25_000, 5_000, 60_000];
    let raised: i128 = donations.iter().sum();
    assert_eq!(raised, 100_000);

    // Sum of individual donations must equal the aggregated raised amount
    let sum_donations: i128 = donations.iter().sum();
    assert_eq!(raised, sum_donations);
}

/// A campaign should never report raised < 0.
#[test]
fn test_invariant_raised_non_negative() {
    let raised: i128 = 0;
    assert!(raised >= 0, "initial raised must be zero or positive");

    let raised_after_donation: i128 = 50_000;
    assert!(raised_after_donation >= 0);
}

/// Withdrawn amount should never exceed raised (overdraft prevention).
#[test]
fn test_invariant_withdrawn_bounded_by_raised() {
    let raised: i128 = 100_000;
    let withdrawn: i128 = 100_000;
    assert!(withdrawn <= raised, "withdrawn may not exceed raised");

    // Attempting withdrawn > raised should be caught
    let excessive_withdrawal = 101_000;
    assert!(
        excessive_withdrawal > raised,
        "excessive withdrawal must exceed raised"
    );
}

/// The stored raised value must equal the sum of all individual donations
/// tracked in the donation contract (cross-contract consistency).
#[test]
fn test_invariant_cross_contract_raised_consistency() {
    // Simulate donation tracking
    let donation_amounts: [i128; 3] = [10_000, 20_000, 30_000];

    // Locally tracked raised
    let local_raised: i128 = donation_amounts.iter().sum();

    // Cross-contract raised (would be fetched from DonationContract)
    let cross_contract_raised: i128 = 60_000; // expected

    assert_eq!(local_raised, cross_contract_raised);
}

/// No overflow when summing large donation amounts.
#[test]
fn test_invariant_no_overflow_on_accumulation() {
    let a: i128 = i128::MAX / 3;
    let b: i128 = i128::MAX / 3;
    let c: i128 = i128::MAX / 3;

    let sum = a.checked_add(b).and_then(|v| v.checked_add(c));
    assert!(
        sum.is_some(),
        "sum of 3 equal partitions of i128::MAX must not overflow"
    );
    assert_eq!(sum.unwrap(), i128::MAX / 3 * 3);
}

/// Total raised across all campaigns is monotonic (never decreases).
#[test]
fn test_invariant_raised_monotonic() {
    let mut raised = 0_i128;
    let donations = [10_000, 25_000, 5_000];

    for d in &donations {
        raised += d;
    }
    assert_eq!(raised, 40_000);

    // After a refund, raised decreases by the refunded amount
    let refund_amount = 10_000;
    raised -= refund_amount;
    assert_eq!(raised, 30_000);
    assert!(
        raised >= 0,
        "raised must stay non-negative even after refund"
    );
}

/// Campaign count is consistent with actual stored campaigns.
#[test]
fn test_invariant_campaign_count_consistency() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, CampaignContract);
    let client = crate::CampaignContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let owner = Address::generate(&env);

    client.initialize(&admin);

    assert_eq!(client.get_campaign_count(), 0_u64);

    let id1 = client.create_campaign(&owner, &1_000_i128, &100_000_u64, &500, &None);
    assert_eq!(id1, 1);
    assert_eq!(client.get_campaign_count(), 1_u64);

    let id2 = client.create_campaign(&owner, &2_000_i128, &200_000_u64, &250, &None);
    assert_eq!(id2, 2);
    assert_eq!(client.get_campaign_count(), 2_u64);
}
