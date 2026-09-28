#![no_std]

#[cfg(test)]
extern crate std;

use shared::guard;
use shared::pause;
use shared::types::{Campaign, CampaignStatus};
use soroban_sdk::{contract, contractimpl, contracttype, Address, BytesN, Env, String, Symbol};

pub mod errors;
pub mod math;
pub mod require;
pub use errors::Error;

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    Initialized,
    Campaign(u64),
    CampaignCount,
    Frozen,
    UnderReview,
    ReviewReason,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContractFrozenEvent {
    pub admin: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContractUnfrozenEvent {
    pub admin: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminChangedEvent {
    pub old_admin: Address,
    pub new_admin: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CampaignUnderReviewEvent {
    pub admin: Address,
    pub reason_hash: BytesN<32>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CampaignReviewClearedEvent {
    pub admin: Address,
}

#[contracttype]
#[derive(Clone)]
pub struct CampaignRegisteredEvent {
    pub campaign_id: u64,
    pub owner: Address,
    pub goal: i128,
    pub deadline: u64,
}

#[contracttype]
#[derive(Clone)]
pub struct CampaignStatusChangedEvent {
    pub campaign_id: u64,
    pub old_status: CampaignStatus,
    pub new_status: CampaignStatus,
}

const MIN_TTL: u32 = 17280; // 1 day in ledgers (assuming 5s ledger time)
const MAX_TTL: u32 = 6312000; // 1 year in ledgers (assuming 5s ledger time)

/// Maximum number of seconds into the future a deadline may be set.
/// 2 years = 2 * 365.25 * 24 * 3600 ≈ 63_115_200 seconds (closes #592).
const MAX_DEADLINE_OFFSET_SECS: u64 = 63_115_200;

/// Maximum byte length for a campaign-related string input (closes #591).
const MAX_STRING_INPUT_LEN: u32 = 512;

/// Smallest donation the campaign contract will credit (closes #759).
/// One stroop: below this a donation cannot be represented in the accounting
/// without rounding, and `DonationTooSmall` is a far better answer than a
/// zero-value ledger entry.
const MIN_DONATION_AMOUNT: i128 = 1;

#[contract]
pub struct CampaignContract;

#[contractimpl]
impl CampaignContract {
    /// Initialize the campaign contract with an admin address.
    /// Must be called once before any other operations.
    pub fn initialize(env: Env, admin: Address) {
        admin.require_auth();
        crate::require!(env, not_initialized, env
            .storage()
            .instance()
            .has(&DataKey::Initialized));
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Initialized, &true);
        env.storage()
            .instance()
            .set(&DataKey::CampaignCount, &0_u64);
        shared::version::seed(&env, env!("CARGO_PKG_VERSION"));
    }

    pub fn get_version(env: Env) -> shared::upgrade::ContractVersion {
        shared::version::query(&env, env!("CARGO_PKG_VERSION"))
    }

    pub fn get_version_metadata(env: Env) -> shared::version::VersionMetadata {
        shared::version::query_metadata(&env, env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
    }

    pub fn is_version_compatible(env: Env, major: u32, minor: u32, patch: u32) -> bool {
        shared::version::query(&env, env!("CARGO_PKG_VERSION")).is_compatible_with(
            &shared::upgrade::ContractVersion {
                major,
                minor,
                patch,
            },
        )
    }

    /// Freeze the campaign contract in an emergency, halting all state-changing activities.
    /// Admin-only. Sets DataKey::Frozen and emits `contract_frozen`.
    pub fn freeze(env: Env) {
        // Guard first: an emergency action must not be re-entrant (#762).
        guard::acquire(&env);
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("contract not initialized");
        admin.require_auth();
        env.storage().instance().set(&DataKey::Frozen, &true);
        guard::release(&env);
        env.events().publish(
            (Symbol::new(&env, "contract_frozen"),),
            ContractFrozenEvent {
                admin: admin.clone(),
            },
        );
    }

    /// Unfreeze the campaign contract, restoring normal state-changing operations.
    /// Admin-only. Clears DataKey::Frozen and emits `contract_unfrozen`.
    pub fn unfreeze(env: Env) {
        guard::acquire(&env);
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("contract not initialized");
        admin.require_auth();
        env.storage().instance().set(&DataKey::Frozen, &false);
        guard::release(&env);
        env.events().publish(
            (Symbol::new(&env, "contract_unfrozen"),),
            ContractUnfrozenEvent {
                admin: admin.clone(),
            },
        );
    }

    /// Query whether the contract is currently frozen.
    pub fn is_frozen(env: Env) -> bool {
        Self::check_frozen(&env)
    }

    /// Flag the campaign contract as under fraud review.
    /// Admin-only. Sets `DataKey::UnderReview` and stores `reason_hash`.
    ///
    /// NOTE: This is currently a manual admin action for Phase 4 fraud scoring readiness.
    /// The actual automated AI fraud-scoring system (Phase 4) is out of scope for this
    /// on-chain contract and will call this hook via an authorized off-chain service/oracle.
    pub fn flag_for_review(env: Env, reason_hash: BytesN<32>) {
        guard::acquire(&env);
        Self::require_not_frozen(&env);
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("contract not initialized");
        admin.require_auth();

        env.storage().instance().set(&DataKey::UnderReview, &true);
        env.storage()
            .instance()
            .set(&DataKey::ReviewReason, &reason_hash);

        guard::release(&env);
        env.events().publish(
            (Symbol::new(&env, "flag_for_review"),),
            CampaignUnderReviewEvent { admin, reason_hash },
        );
    }

    /// Clear the fraud review flag, restoring normal withdrawal operations.
    /// Admin-only. Clears `DataKey::UnderReview` and removes `reason_hash`.
    ///
    /// NOTE: This is a manual admin action. Once investigation concludes or an
    /// off-chain review clears the account, an admin clears this flag.
    pub fn clear_review_flag(env: Env) {
        guard::acquire(&env);
        Self::require_not_frozen(&env);
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("contract not initialized");
        admin.require_auth();

        env.storage().instance().set(&DataKey::UnderReview, &false);
        env.storage().instance().remove(&DataKey::ReviewReason);

        guard::release(&env);
        env.events().publish(
            (Symbol::new(&env, "clear_review_flag"),),
            CampaignReviewClearedEvent { admin },
        );
    }

    /// Query whether the contract is currently flagged under fraud review.
    pub fn is_under_review(env: Env) -> bool {
        Self::check_under_review(&env)
    }

    /// Query the 32-byte reason hash associated with the current fraud review, if any.
    pub fn get_review_reason(env: Env) -> Option<BytesN<32>> {
        env.storage().instance().get(&DataKey::ReviewReason)
    }

    /// Pause the contract, blocking all state-changing operations.
    pub fn pause(env: Env, admin: Address) {
        Self::require_not_frozen(&env);
        Self::ensure_admin(&env, &admin);
        pause::pause(&env, &admin);
    }

    /// Unpause the contract, restoring normal operations.
    pub fn unpause(env: Env, admin: Address) {
        Self::require_not_frozen(&env);
        Self::ensure_admin(&env, &admin);
        pause::unpause(&env, &admin);
    }

    /// Create a new fundraising campaign.
    /// Returns the newly assigned campaign ID.
    /// Closes #591 – no string input in this function, validated at caller.
    /// Closes #592 – validates deadline does not exceed 2 years from now.
    pub fn create_campaign(
        env: Env,
        owner: Address,
        goal: i128,
        deadline: u64,
        fee_bps: u32,
        platform_wallet: Option<Address>,
    ) -> u64 {
        // Guard first, before any read a re-entrant call could observe as
        // half-applied (#762).
        guard::acquire(&env);
        Self::require_not_frozen(&env);
        pause::require_not_paused(&env);
        owner.require_auth();
        if fee_bps > 1000 {
            panic!("fee_bps must not exceed 1000");
        }
        // ── Typed input validation (closes #759) ──────────────────────────
        crate::require!(env, valid_goal, goal);
        // ── Deadline upper bound (closes #592) and typed lower bound (#759) ──
        let now = env.ledger().timestamp();
        crate::require!(env, valid_end_time, now, deadline, MAX_DEADLINE_OFFSET_SECS);
        let id = Self::next_campaign_id(&env);
        let campaign = Campaign {
            id,
            owner: owner.clone(),
            goal,
            raised: 0,
            status: CampaignStatus::Active,
            deadline,
            fee_bps,
            platform_wallet,
        };
        env.storage()
            .persistent()
            .set(&DataKey::Campaign(id), &campaign);
        Self::bump_campaign_ttl(env.clone(), id);
        guard::release(&env);
        env.events().publish(
            (Symbol::new(&env, "campaign_registered"),),
            CampaignRegisteredEvent {
                campaign_id: id,
                owner,
                goal,
                deadline,
            },
        );
        id
    }

    /// Get campaign details by ID.
    pub fn get_campaign(env: Env, campaign_id: u64) -> Option<Campaign> {
        env.storage()
            .persistent()
            .get(&DataKey::Campaign(campaign_id))
    }

    /// Update the status of a campaign. Emits a `campaign_status_changed` event
    /// with both old and new status values.
    pub fn update_campaign_status(
        env: Env,
        admin: Address,
        campaign_id: u64,
        new_status: CampaignStatus,
    ) {
        // Guard first (#762). Note: `approve_campaign` / `suspend_campaign`
        // delegate here, so they are deliberately NOT guarded themselves — the
        // guard is not re-entrant and would reject the inner call.
        guard::acquire(&env);
        Self::require_not_frozen(&env);
        admin.require_auth();
        Self::ensure_admin(&env, &admin);
        let mut campaign = Self::get_campaign(env.clone(), campaign_id).unwrap();
        let old_status = campaign.status;
        campaign.status = new_status;
        env.storage()
            .persistent()
            .set(&DataKey::Campaign(campaign_id), &campaign);
        guard::release(&env);
        env.events().publish(
            (Symbol::new(&env, "campaign_status_changed"),),
            CampaignStatusChangedEvent {
                campaign_id,
                old_status,
                new_status,
            },
        );
    }

    /// Increment the raised amount for a campaign. Called via cross-contract
    /// call from the Donation contract after a successful donation.
    ///
    /// Rejects a closed or non-active campaign (`CampaignEnded` /
    /// `CampaignNotActive`), a sub-minimum donation (`DonationTooSmall`), and
    /// a call made before `initialize` (`NotInitialized`).
    pub fn update_raised(env: Env, campaign_id: u64, amount: i128) {
        // Guard first (#762).
        guard::acquire(&env);
        Self::require_not_frozen(&env);
        pause::require_not_paused(&env);
        crate::require!(env, initialized, env
            .storage()
            .instance()
            .has(&DataKey::Initialized));
        crate::require!(env, donation_minimum, amount, MIN_DONATION_AMOUNT);
        let mut campaign = env
            .storage()
            .persistent()
            .get::<DataKey, Campaign>(&DataKey::Campaign(campaign_id))
            .unwrap();
        crate::require!(env, campaign_open, campaign.status);
        // Checked addition: a silent wrap of `raised` would report a balance
        // the campaign does not hold (#760).
        campaign.raised = math::add(&env, campaign.raised, amount);
        env.storage()
            .persistent()
            .set(&DataKey::Campaign(campaign_id), &campaign);
        Self::bump_campaign_ttl(env.clone(), campaign_id);
        guard::release(&env);
    }

    /// Finalize a withdrawal for a campaign.
    /// Deducts `amount` from `campaign.raised`.
    /// Blocked with panic `UnderReview` if the contract is flagged for fraud review.
    /// Also blocked if the contract is frozen (`ContractFrozen`) or paused.
    pub fn finalize_withdrawal(env: Env, campaign_id: u64, amount: i128) {
        // Guard first (#762).
        guard::acquire(&env);
        Self::require_not_frozen(&env);
        Self::require_not_under_review(&env);
        pause::require_not_paused(&env);

        let mut campaign =
            Self::get_campaign(env.clone(), campaign_id).expect("campaign not found");
        campaign.owner.require_auth();

        if amount <= 0 {
            panic!("withdrawal amount must be positive");
        }
        // The campaign's recorded balance must cover the payout, checked
        // before any state is mutated so a doomed withdrawal can never leave a
        // half-applied debit (#759 / #764).
        crate::require!(env, contract_balance, campaign.raised, amount);

        // Checked subtraction: an underflow here would mean paying out funds
        // the campaign never raised (#760).
        campaign.raised = math::sub(&env, campaign.raised, amount);
        env.storage()
            .persistent()
            .set(&DataKey::Campaign(campaign_id), &campaign);
        Self::bump_campaign_ttl(env.clone(), campaign_id);

        // Platform fee on the drawn amount, computed with checked maths so
        // `fee + net` is exactly `amount` (#760).
        let (fee, _net) = math::fee_split(&env, amount, campaign.fee_bps);

        guard::release(&env);
        env.events().publish(
            (Symbol::new(&env, "withdrawal_finalized"),),
            (campaign_id, amount, fee, campaign.owner, env.ledger().timestamp()),
        );
    }

    /// Approve a campaign, moving it to Active status.
    pub fn approve_campaign(env: Env, admin: Address, campaign_id: u64) {
        Self::require_not_frozen(&env);
        Self::update_campaign_status(env, admin, campaign_id, CampaignStatus::Active);
    }

    /// Reject a campaign, moving it to Rejected status.
    /// Closes #591 – validates reason length.
    pub fn reject_campaign(env: Env, admin: Address, campaign_id: u64, reason: String) {
        guard::acquire(&env);
        Self::require_not_frozen(&env);
        pause::require_not_paused(&env);
        admin.require_auth();
        Self::ensure_admin(&env, &admin);
        // ── Input length validation (closes #591) ──────────────────────────
        if reason.len() > MAX_STRING_INPUT_LEN {
            panic!("reason exceeds maximum allowed length");
        }
        let mut campaign = Self::get_campaign(env.clone(), campaign_id).unwrap();
        let old_status = campaign.status;
        campaign.status = CampaignStatus::Rejected;
        env.storage()
            .persistent()
            .set(&DataKey::Campaign(campaign_id), &campaign);
        guard::release(&env);
        env.events().publish(
            (Symbol::new(&env, "campaign_status_changed"),),
            CampaignStatusChangedEvent {
                campaign_id,
                old_status,
                new_status: CampaignStatus::Rejected,
            },
        );
        let _ = reason;
    }

    /// Suspend a campaign, moving it to Suspended status.
    pub fn suspend_campaign(env: Env, admin: Address, campaign_id: u64) {
        Self::require_not_frozen(&env);
        Self::update_campaign_status(env, admin, campaign_id, CampaignStatus::Suspended);
    }

    /// Get the total number of campaigns created.
    pub fn get_campaign_count(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::CampaignCount)
            .unwrap_or(0_u64)
    }

    /// Return the currently active admin address.
    pub fn get_admin(env: Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("contract not initialized")
    }

    /// Transfer admin rights to a new address with dual authorization.
    /// Current admin authorization and new admin authorization are both required.
    /// Emits `admin_changed` event and maintains exactly one admin in storage.
    pub fn set_admin(env: Env, new_admin: Address) {
        guard::acquire(&env);
        Self::require_not_frozen(&env);
        pause::require_not_paused(&env);

        let current_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("contract not initialized");

        current_admin.require_auth();
        new_admin.require_auth();

        env.storage().instance().set(&DataKey::Admin, &new_admin);
        guard::release(&env);

        env.events().publish(
            (Symbol::new(&env, "admin_changed"),),
            AdminChangedEvent {
                old_admin: current_admin,
                new_admin,
            },
        );
    }

    /// Transfer admin privileges to a new address.
    pub fn transfer_admin(env: Env, current_admin: Address, new_admin: Address) {
        guard::acquire(&env);
        Self::require_not_frozen(&env);
        pause::require_not_paused(&env);
        current_admin.require_auth();
        Self::ensure_admin(&env, &current_admin);
        env.storage().instance().set(&DataKey::Admin, &new_admin);
        guard::release(&env);
    }

    /// Upgrade the contract to a new WASM implementation.
    pub fn upgrade(env: Env, admin: Address, new_wasm_hash: BytesN<32>) {
        Self::require_not_frozen(&env);
        admin.require_auth();
        Self::ensure_admin(&env, &admin);
        env.deployer().update_current_contract_wasm(new_wasm_hash);
    }

    /// Bumps the TTL of a campaign to ensure it doesn't expire.
    /// Archive (delete) a campaign record.
    /// Only the admin can archive a campaign, and only if its status is
    /// Completed, Rejected, Cancelled, or Suspended (i.e., no active funds
    /// in flight).
    pub fn archive_campaign(env: Env, admin: Address, campaign_id: u64) {
        guard::acquire(&env);
        Self::require_not_frozen(&env);
        admin.require_auth();
        Self::ensure_admin(&env, &admin);
        let campaign = Self::get_campaign(env.clone(), campaign_id).unwrap();
        match campaign.status {
            CampaignStatus::Active | CampaignStatus::Pending => {
                panic!("cannot archive an active or pending campaign");
            }
            _ => {}
        }
        env.storage()
            .persistent()
            .remove(&DataKey::Campaign(campaign_id));
        guard::release(&env);
        env.events()
            .publish((Symbol::new(&env, "campaign_archived"),), (campaign_id,));
    }

    pub fn get_fee_config(env: Env, campaign_id: u64) -> (u32, Option<Address>) {
        let campaign = Self::get_campaign(env.clone(), campaign_id).unwrap();
        (campaign.fee_bps, campaign.platform_wallet)
    }

    pub fn bump_campaign_ttl(env: Env, campaign_id: u64) {
        Self::require_not_frozen(&env);
        let key = DataKey::Campaign(campaign_id);
        env.storage()
            .persistent()
            .extend_ttl(&key, MIN_TTL, MAX_TTL);
    }

    fn check_frozen(env: &Env) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::Frozen)
            .unwrap_or(false)
    }

    fn require_not_frozen(env: &Env) {
        crate::require!(env, not_frozen, Self::check_frozen(env));
    }

    fn check_under_review(env: &Env) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::UnderReview)
            .unwrap_or(false)
    }

    fn require_not_under_review(env: &Env) {
        if Self::check_under_review(env) {
            panic!("UnderReview");
        }
    }

    fn ensure_admin(env: &Env, admin: &Address) {
        let stored_admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        crate::require!(env, authorized, stored_admin == *admin);
    }

    fn next_campaign_id(env: &Env) -> u64 {
        let current: u64 = env
            .storage()
            .instance()
            .get(&DataKey::CampaignCount)
            .unwrap_or(0_u64);
        // Checked: a wrapped campaign counter would hand out a duplicate id (#760).
        let next_id: u64 = math::add_u64(env, current, 1);
        env.storage()
            .instance()
            .set(&DataKey::CampaignCount, &next_id);
        next_id
    }

    // ── Health monitoring (#678) and gradual rollout (#684) ──────────────
    pub fn health_check(env: Env) -> shared::health::HealthReport {
        let report = shared::health::health_check(&env);
        if report.anomaly {
            shared::rollout::maybe_auto_rollback(&env);
        }
        report
    }
    pub fn get_health_metrics(env: Env) -> shared::health::HealthMetrics {
        shared::health::get_metrics(&env)
    }
    pub fn get_sla_targets(env: Env) -> shared::health::SlaTargets {
        let _ = env;
        shared::health::sla_targets()
    }
    pub fn set_alert_config(env: Env, admin: Address, config: shared::health::AlertConfig) {
        Self::require_not_frozen(&env);
        admin.require_auth();
        shared::health::set_alert_config(&env, config);
    }
    pub fn get_alert_config(env: Env) -> shared::health::AlertConfig {
        shared::health::get_alert_config(&env)
    }
    pub fn detect_anomaly(env: Env) -> bool {
        shared::health::detect_anomaly(&env)
    }
    pub fn report_ok(env: Env, admin: Address) {
        Self::require_not_frozen(&env);
        admin.require_auth();
        shared::health::record_ok(&env);
    }
    pub fn report_error(env: Env, admin: Address) {
        Self::require_not_frozen(&env);
        admin.require_auth();
        shared::health::record_error(&env);
    }
    pub fn set_feature_flag(env: Env, admin: Address, flag: soroban_sdk::Symbol, enabled: bool) {
        Self::require_not_frozen(&env);
        admin.require_auth();
        shared::rollout::set_feature_flag(&env, &flag, enabled);
    }
    pub fn is_feature_enabled(env: Env, flag: soroban_sdk::Symbol) -> bool {
        shared::rollout::is_feature_enabled(&env, &flag)
    }
    pub fn set_canary_deployment(
        env: Env,
        admin: Address,
        canary: Address,
        stable: Address,
        canary_bps: u32,
    ) {
        Self::require_not_frozen(&env);
        admin.require_auth();
        shared::rollout::set_canary_deployment(&env, canary, stable, canary_bps);
    }
    pub fn route_to_canary(env: Env, caller: Address) -> bool {
        shared::rollout::route_to_canary(&env, &caller)
    }
    pub fn get_rollout_state(env: Env) -> shared::rollout::RolloutState {
        shared::rollout::get_state(&env)
    }
    pub fn set_rollback_trigger(env: Env, admin: Address, error_bps: u32) {
        Self::require_not_frozen(&env);
        admin.require_auth();
        shared::rollout::set_rollback_trigger(&env, error_bps);
    }
    pub fn should_rollback(env: Env) -> bool {
        shared::rollout::should_rollback(&env)
    }
    pub fn trigger_rollback(env: Env, admin: Address) {
        Self::require_not_frozen(&env);
        admin.require_auth();
        shared::rollout::trigger_rollback(&env, &admin);
    }
}

#[cfg(test)]
mod invariant_tests;
#[cfg(test)]
mod negative_tests;
#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env};

    #[test]
    fn campaign_admin_and_status_flow() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, CampaignContract);
        let client = CampaignContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let owner = Address::generate(&env);

        client.initialize(&admin);
        let campaign_id = client.create_campaign(&owner, &1_000_i128, &2_000_u64, &500, &None);
        let campaign = client.get_campaign(&campaign_id).unwrap();

        assert_eq!(campaign.owner, owner);
        assert_eq!(campaign.goal, 1_000_i128);
        assert_eq!(campaign.status, CampaignStatus::Active);
        assert_eq!(campaign.fee_bps, 500);
        assert_eq!(campaign.platform_wallet, None);
        assert_eq!(client.get_campaign_count(), 1_u64);

        client.suspend_campaign(&admin, &campaign_id);
        let suspended = client.get_campaign(&campaign_id).unwrap();
        assert_eq!(suspended.status, CampaignStatus::Suspended);

        client.reject_campaign(&admin, &campaign_id, &String::from_str(&env, "spam"));
        let rejected = client.get_campaign(&campaign_id).unwrap();
        assert_eq!(rejected.status, CampaignStatus::Rejected);
    }

    #[test]
    fn pause_blocks_state_mutations() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, CampaignContract);
        let client = CampaignContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let owner = Address::generate(&env);

        client.initialize(&admin);
        client.pause(&admin);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.create_campaign(&owner, &1_000_i128, &2_000_u64, &500, &None);
        }));
        assert!(result.is_err());

        let new_admin = Address::generate(&env);
        let res_set_admin = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.set_admin(&new_admin);
        }));
        assert!(res_set_admin.is_err());

        client.unpause(&admin);
        let campaign_id = client.create_campaign(&owner, &1_000_i128, &2_000_u64, &500, &None);
        assert_eq!(campaign_id, 1);
    }

    #[test]
    fn get_version_matches_cargo_semver() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, CampaignContract);
        let client = CampaignContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);

        client.initialize(&admin);
        let v = client.get_version();
        assert_eq!(v.major, 0);
        assert_eq!(v.minor, 1);
        assert_eq!(v.patch, 0);
        assert!(client.is_version_compatible(&0, &1, &0));
        assert!(!client.is_version_compatible(&0, &2, &0));
        let meta = client.get_version_metadata();
        assert_eq!(meta.storage_schema, 1);
        assert_eq!(meta.min_compatible.minor, 1);
    }

    #[test]
    fn emergency_freeze_and_unfreeze_flow() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, CampaignContract);
        let client = CampaignContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let owner = Address::generate(&env);

        client.initialize(&admin);
        assert!(!client.is_frozen());

        // Freeze contract
        client.freeze();
        assert!(client.is_frozen());

        // All state-changing operations must panic ContractFrozen
        let res_create = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.create_campaign(&owner, &1_000_i128, &2_000_u64, &500, &None);
        }));
        assert!(res_create.is_err());

        let res_pause = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.pause(&admin);
        }));
        assert!(res_pause.is_err());

        let res_unpause = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.unpause(&admin);
        }));
        assert!(res_unpause.is_err());

        let res_status = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.update_campaign_status(&admin, &1, &CampaignStatus::Completed);
        }));
        assert!(res_status.is_err());

        let res_raised = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.update_raised(&1, &500);
        }));
        assert!(res_raised.is_err());

        let res_approve = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.approve_campaign(&admin, &1);
        }));
        assert!(res_approve.is_err());

        let res_reject = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.reject_campaign(&admin, &1, &String::from_str(&env, "bad"));
        }));
        assert!(res_reject.is_err());

        let res_suspend = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.suspend_campaign(&admin, &1);
        }));
        assert!(res_suspend.is_err());

        let new_admin = Address::generate(&env);
        let res_transfer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.transfer_admin(&admin, &new_admin);
        }));
        assert!(res_transfer.is_err());

        let res_set_admin = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.set_admin(&new_admin);
        }));
        assert!(res_set_admin.is_err());

        let res_upgrade = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.upgrade(&admin, &BytesN::from_array(&env, &[0u8; 32]));
        }));
        assert!(res_upgrade.is_err());

        let res_archive = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.archive_campaign(&admin, &1);
        }));
        assert!(res_archive.is_err());

        let res_ttl = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.bump_campaign_ttl(&1);
        }));
        assert!(res_ttl.is_err());

        let res_report_ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.report_ok(&admin);
        }));
        assert!(res_report_ok.is_err());

        let res_report_err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.report_error(&admin);
        }));
        assert!(res_report_err.is_err());

        let res_trigger_rb = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.trigger_rollback(&admin);
        }));
        assert!(res_trigger_rb.is_err());

        // Unfreeze contract
        client.unfreeze();
        assert!(!client.is_frozen());

        // After unfreeze, mutations succeed
        let campaign_id = client.create_campaign(&owner, &1_000_i128, &2_000_u64, &500, &None);
        assert_eq!(campaign_id, 1);
        assert_eq!(client.get_campaign_count(), 1);
    }

    #[test]
    fn frozen_contract_allows_read_only_queries() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, CampaignContract);
        let client = CampaignContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let owner = Address::generate(&env);

        client.initialize(&admin);
        let campaign_id = client.create_campaign(&owner, &5_000_i128, &2_000_u64, &250, &None);

        client.freeze();
        assert!(client.is_frozen());

        // Views must succeed without panicking while frozen
        let campaign = client.get_campaign(&campaign_id).unwrap();
        assert_eq!(campaign.goal, 5_000_i128);
        assert_eq!(client.get_campaign_count(), 1);
        assert_eq!(client.get_fee_config(&campaign_id), (250, None));
        let ver = client.get_version();
        assert_eq!(ver.major, 0);
        let meta = client.get_version_metadata();
        assert_eq!(meta.storage_schema, 1);
        assert!(client.is_version_compatible(&0, &1, &0));
        let metrics = client.get_health_metrics();
        assert_eq!(metrics.ok_count, 0);
    }

    #[test]
    fn freeze_and_unfreeze_emit_events() {
        use soroban_sdk::testutils::Events as _;
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, CampaignContract);
        let client = CampaignContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);

        client.initialize(&admin);

        let before_freeze = env.events().all().len();
        client.freeze();
        let after_freeze = env.events().all().len();
        assert!(after_freeze > before_freeze);

        let before_unfreeze = env.events().all().len();
        client.unfreeze();
        let after_unfreeze = env.events().all().len();
        assert!(after_unfreeze > before_unfreeze);
    }

    #[test]
    fn set_admin_successful_rotation_and_single_admin() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, CampaignContract);
        let client = CampaignContractClient::new(&env, &contract_id);
        let admin1 = Address::generate(&env);
        let admin2 = Address::generate(&env);
        let admin3 = Address::generate(&env);

        client.initialize(&admin1);
        assert_eq!(client.get_admin(), admin1);

        // Rotate to admin2
        client.set_admin(&admin2);
        assert_eq!(client.get_admin(), admin2);

        // admin2 has admin powers
        client.pause(&admin2);
        client.unpause(&admin2);

        // admin1 no longer has admin powers
        let res_old_admin = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.pause(&admin1);
        }));
        assert!(res_old_admin.is_err());

        // Rotate again from admin2 to admin3
        client.set_admin(&admin3);
        assert_eq!(client.get_admin(), admin3);

        // admin2 no longer has admin powers
        let res_admin2 = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.pause(&admin2);
        }));
        assert!(res_admin2.is_err());
    }

    #[test]
    fn set_admin_emits_event() {
        use soroban_sdk::testutils::Events as _;
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, CampaignContract);
        let client = CampaignContractClient::new(&env, &contract_id);
        let admin1 = Address::generate(&env);
        let admin2 = Address::generate(&env);

        client.initialize(&admin1);

        let before = env.events().all().len();
        client.set_admin(&admin2);
        let after = env.events().all().len();
        assert!(after > before);

        use soroban_sdk::FromVal;
        let events = env.events().all();
        let (_, topics, data) = events.last().unwrap();
        let topic_sym = Symbol::from_val(&env, &topics.get(0).unwrap());
        assert_eq!(topic_sym, Symbol::new(&env, "admin_changed"));
        let event_payload = AdminChangedEvent::from_val(&env, &data);
        assert_eq!(event_payload.old_admin, admin1);
        assert_eq!(event_payload.new_admin, admin2);
    }

    #[test]
    fn set_admin_requires_both_auths() {
        use soroban_sdk::testutils::MockAuth;
        use soroban_sdk::testutils::MockAuthInvoke;
        use soroban_sdk::IntoVal;

        let env = Env::default();
        let contract_id = env.register_contract(None, CampaignContract);
        let client = CampaignContractClient::new(&env, &contract_id);
        let admin1 = Address::generate(&env);
        let admin2 = Address::generate(&env);

        env.mock_all_auths();
        client.initialize(&admin1);

        // 1. If only admin1 authorizes (new_admin does not authorize), set_admin must panic
        env.mock_auths(&[MockAuth {
            address: &admin1,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "set_admin",
                args: (admin2.clone(),).into_val(&env),
                sub_invokes: &[],
            },
        }]);
        let res_no_new_auth = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.set_admin(&admin2);
        }));
        assert!(res_no_new_auth.is_err());

        // 2. If only admin2 authorizes (current admin does not authorize), set_admin must panic
        env.mock_auths(&[MockAuth {
            address: &admin2,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "set_admin",
                args: (admin2.clone(),).into_val(&env),
                sub_invokes: &[],
            },
        }]);
        let res_no_current_auth = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.set_admin(&admin2);
        }));
        assert!(res_no_current_auth.is_err());

        // 3. If both authorize, set_admin succeeds
        env.mock_auths(&[
            MockAuth {
                address: &admin1,
                invoke: &MockAuthInvoke {
                    contract: &contract_id,
                    fn_name: "set_admin",
                    args: (admin2.clone(),).into_val(&env),
                    sub_invokes: &[],
                },
            },
            MockAuth {
                address: &admin2,
                invoke: &MockAuthInvoke {
                    contract: &contract_id,
                    fn_name: "set_admin",
                    args: (admin2.clone(),).into_val(&env),
                    sub_invokes: &[],
                },
            },
        ]);
        client.set_admin(&admin2);
        assert_eq!(client.get_admin(), admin2);
    }

    #[contract]
    pub struct MockMultiSigWallet;

    #[contractimpl]
    impl MockMultiSigWallet {
        pub fn execute(env: Env, target_contract: Address, call_freeze: bool) {
            let client = CampaignContractClient::new(&env, &target_contract);
            if call_freeze {
                client.freeze();
            } else {
                client.unfreeze();
            }
        }
    }

    #[test]
    fn test_admin_multisig_readiness_with_contract_wallet() {
        let env = Env::default();
        env.mock_all_auths();
        let campaign_id = env.register_contract(None, CampaignContract);
        let campaign_client = CampaignContractClient::new(&env, &campaign_id);

        // Register multi-sig contract as admin
        let multisig_wallet_id = env.register_contract(None, MockMultiSigWallet);
        let multisig_client = MockMultiSigWalletClient::new(&env, &multisig_wallet_id);

        // Initialize campaign with multi-sig contract address as admin
        campaign_client.initialize(&multisig_wallet_id);
        assert_eq!(campaign_client.get_admin(), multisig_wallet_id);
        assert!(!campaign_client.is_frozen());

        // Multi-sig contract executes freeze on campaign
        multisig_client.execute(&campaign_id, &true);
        assert!(campaign_client.is_frozen());

        // Multi-sig contract executes unfreeze on campaign
        multisig_client.execute(&campaign_id, &false);
        assert!(!campaign_client.is_frozen());
    }

    #[test]
    fn test_admin_multisig_simulated_threshold_authorization() {
        use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};
        use soroban_sdk::IntoVal;

        let env = Env::default();
        let contract_id = env.register_contract(None, CampaignContract);
        let client = CampaignContractClient::new(&env, &contract_id);
        let multisig_admin = Address::generate(&env);
        let signer1 = Address::generate(&env);
        let signer2 = Address::generate(&env);

        env.mock_all_auths();
        client.initialize(&multisig_admin);
        assert_eq!(client.get_admin(), multisig_admin);

        // 1. Threshold not reached: only a single signer authorizes without satisfying admin auth
        env.mock_auths(&[MockAuth {
            address: &signer1,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "freeze",
                args: ().into_val(&env),
                sub_invokes: &[],
            },
        }]);
        let res_single_signer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.freeze();
        }));
        assert!(res_single_signer.is_err());
        assert!(!client.is_frozen());

        // 2. Threshold satisfied: simulated multi-sig transaction with multisig_admin authorized
        // (representing valid aggregate signatures from signer1 + signer2 meeting medium threshold)
        env.mock_auths(&[
            MockAuth {
                address: &multisig_admin,
                invoke: &MockAuthInvoke {
                    contract: &contract_id,
                    fn_name: "freeze",
                    args: ().into_val(&env),
                    sub_invokes: &[],
                },
            },
            MockAuth {
                address: &signer1,
                invoke: &MockAuthInvoke {
                    contract: &contract_id,
                    fn_name: "freeze",
                    args: ().into_val(&env),
                    sub_invokes: &[],
                },
            },
            MockAuth {
                address: &signer2,
                invoke: &MockAuthInvoke {
                    contract: &contract_id,
                    fn_name: "freeze",
                    args: ().into_val(&env),
                    sub_invokes: &[],
                },
            },
        ]);
        client.freeze();
        assert!(client.is_frozen());

        // 3. Unfreeze with multi-sig auth
        env.mock_auths(&[MockAuth {
            address: &multisig_admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "unfreeze",
                args: ().into_val(&env),
                sub_invokes: &[],
            },
        }]);
        client.unfreeze();
        assert!(!client.is_frozen());
    }

    #[test]
    fn test_admin_multisig_rotation_between_multisig_accounts() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, CampaignContract);
        let client = CampaignContractClient::new(&env, &contract_id);
        let multisig_admin_1 = Address::generate(&env);
        let multisig_admin_2 = Address::generate(&env);

        client.initialize(&multisig_admin_1);
        assert_eq!(client.get_admin(), multisig_admin_1);

        // Rotate from multi-sig 1 to multi-sig 2
        client.set_admin(&multisig_admin_2);
        assert_eq!(client.get_admin(), multisig_admin_2);

        // multisig_admin_2 has admin control
        client.pause(&multisig_admin_2);
        client.unpause(&multisig_admin_2);

        // multisig_admin_1 no longer has admin control
        let res_old = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.pause(&multisig_admin_1);
        }));
        assert!(res_old.is_err());
    }

    #[test]
    fn test_fraud_review_blocks_finalize_withdrawal() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, CampaignContract);
        let client = CampaignContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let owner = Address::generate(&env);

        client.initialize(&admin);
        let campaign_id = client.create_campaign(&owner, &10_000_i128, &2_000_u64, &500, &None);
        client.update_raised(&campaign_id, &5_000_i128);

        // Before flagging, not under review
        assert!(!client.is_under_review());
        assert_eq!(client.get_review_reason(), None);

        // Withdrawal before flag succeeds
        client.finalize_withdrawal(&campaign_id, &1_000_i128);
        let campaign = client.get_campaign(&campaign_id).unwrap();
        assert_eq!(campaign.raised, 4_000_i128);

        // Flag for review
        let reason_hash = BytesN::from_array(&env, &[7u8; 32]);
        client.flag_for_review(&reason_hash);
        assert!(client.is_under_review());
        assert_eq!(client.get_review_reason(), Some(reason_hash));

        // finalize_withdrawal is BLOCKED while under review
        let res_blocked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.finalize_withdrawal(&campaign_id, &1_000_i128);
        }));
        assert!(res_blocked.is_err());

        // Read-only queries still work while under review
        let c = client.get_campaign(&campaign_id).unwrap();
        assert_eq!(c.raised, 4_000_i128);
        assert_eq!(client.get_campaign_count(), 1);

        // Clear review flag
        client.clear_review_flag();
        assert!(!client.is_under_review());
        assert_eq!(client.get_review_reason(), None);

        // Withdrawal after clearing review flag succeeds
        client.finalize_withdrawal(&campaign_id, &1_000_i128);
        let updated = client.get_campaign(&campaign_id).unwrap();
        assert_eq!(updated.raised, 3_000_i128);
    }

    #[test]
    fn test_fraud_review_admin_authorization() {
        use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};
        use soroban_sdk::IntoVal;

        let env = Env::default();
        let contract_id = env.register_contract(None, CampaignContract);
        let client = CampaignContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let non_admin = Address::generate(&env);

        env.mock_all_auths();
        client.initialize(&admin);

        let reason_hash = BytesN::from_array(&env, &[9u8; 32]);

        // Non-admin cannot flag for review
        env.mock_auths(&[MockAuth {
            address: &non_admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "flag_for_review",
                args: (reason_hash.clone(),).into_val(&env),
                sub_invokes: &[],
            },
        }]);
        let res_non_admin_flag = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.flag_for_review(&reason_hash);
        }));
        assert!(res_non_admin_flag.is_err());
        assert!(!client.is_under_review());

        // Admin flags successfully
        env.mock_auths(&[MockAuth {
            address: &admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "flag_for_review",
                args: (reason_hash.clone(),).into_val(&env),
                sub_invokes: &[],
            },
        }]);
        client.flag_for_review(&reason_hash);
        assert!(client.is_under_review());

        // Non-admin cannot clear review flag
        env.mock_auths(&[MockAuth {
            address: &non_admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "clear_review_flag",
                args: ().into_val(&env),
                sub_invokes: &[],
            },
        }]);
        let res_non_admin_clear = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.clear_review_flag();
        }));
        assert!(res_non_admin_clear.is_err());
        assert!(client.is_under_review());

        // Admin clears flag successfully
        env.mock_auths(&[MockAuth {
            address: &admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "clear_review_flag",
                args: ().into_val(&env),
                sub_invokes: &[],
            },
        }]);
        client.clear_review_flag();
        assert!(!client.is_under_review());
    }

    #[test]
    fn test_fraud_review_events_emission() {
        use soroban_sdk::testutils::Events as _;
        use soroban_sdk::FromVal;

        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, CampaignContract);
        let client = CampaignContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);

        client.initialize(&admin);

        let reason_hash = BytesN::from_array(&env, &[5u8; 32]);
        let before_flag = env.events().all().len();
        client.flag_for_review(&reason_hash);
        let after_flag = env.events().all().len();
        assert!(after_flag > before_flag);

        let events = env.events().all();
        let (_, topics, data) = events.last().unwrap();
        let topic_sym = Symbol::from_val(&env, &topics.get(0).unwrap());
        assert_eq!(topic_sym, Symbol::new(&env, "flag_for_review"));
        let event_payload = CampaignUnderReviewEvent::from_val(&env, &data);
        assert_eq!(event_payload.admin, admin);
        assert_eq!(event_payload.reason_hash, reason_hash);

        let before_clear = env.events().all().len();
        client.clear_review_flag();
        let after_clear = env.events().all().len();
        assert!(after_clear > before_clear);

        let events_clear = env.events().all();
        let (_, topics_clear, data_clear) = events_clear.last().unwrap();
        let topic_clear_sym = Symbol::from_val(&env, &topics_clear.get(0).unwrap());
        assert_eq!(topic_clear_sym, Symbol::new(&env, "clear_review_flag"));
        let event_clear_payload = CampaignReviewClearedEvent::from_val(&env, &data_clear);
        assert_eq!(event_clear_payload.admin, admin);
    }

    #[test]
    fn test_fraud_review_interaction_with_freeze() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, CampaignContract);
        let client = CampaignContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let owner = Address::generate(&env);

        client.initialize(&admin);
        let campaign_id = client.create_campaign(&owner, &10_000_i128, &2_000_u64, &500, &None);
        client.update_raised(&campaign_id, &5_000_i128);

        client.freeze();

        // Flagging while frozen panics ContractFrozen
        let reason_hash = BytesN::from_array(&env, &[1u8; 32]);
        let res_flag = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.flag_for_review(&reason_hash);
        }));
        assert!(res_flag.is_err());

        // finalize_withdrawal while frozen panics ContractFrozen
        let res_wd = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.finalize_withdrawal(&campaign_id, &1_000_i128);
        }));
        assert!(res_wd.is_err());
    }
}
