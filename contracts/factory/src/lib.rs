#![no_std]

#[cfg(test)]
extern crate std;

use soroban_sdk::{contract, contractimpl, contracttype, token, Address, BytesN, Env, Symbol};

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    Initialized,
    CampaignWasmHash,
    DeploymentFee,
    Treasury,
}

#[contracttype]
#[derive(Clone)]
pub struct CampaignDeployedEvent {
    pub creator: Address,
    pub contract_address: Address,
    pub salt: BytesN<32>,
}

#[contract]
pub struct CampaignFactoryContract;

#[contractimpl]
impl CampaignFactoryContract {
    /// Initialize the factory with an admin and the platform treasury that
    /// receives deployment fees. Must be called once before any other
    /// operations.
    pub fn initialize(env: Env, admin: Address, treasury: Address) {
        admin.require_auth();
        if env.storage().instance().has(&DataKey::Initialized) {
            panic!("already initialized");
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Treasury, &treasury);
        env.storage().instance().set(&DataKey::DeploymentFee, &0_i128);
        env.storage().instance().set(&DataKey::Initialized, &true);
        shared::version::seed(&env, env!("CARGO_PKG_VERSION"));
    }

    shared::impl_semver_queries!();

    /// Store the WASM hash used for new campaign deployments.
    /// Admin-only. Already-deployed campaign instances keep the code they
    /// were originally deployed with.
    pub fn update_wasm_hash(env: Env, admin: Address, new_hash: BytesN<32>) {
        admin.require_auth();
        Self::ensure_admin(&env, &admin);
        env.storage()
            .instance()
            .set(&DataKey::CampaignWasmHash, &new_hash);
    }

    /// Return the WASM hash currently used for new deployments, if set.
    pub fn get_wasm_hash(env: Env) -> Option<BytesN<32>> {
        env.storage().instance().get(&DataKey::CampaignWasmHash)
    }

    /// Set the XLM fee (in stroops) charged per campaign deployment.
    /// Admin-only. The fee must not be negative.
    pub fn set_deployment_fee(env: Env, admin: Address, fee: i128) {
        admin.require_auth();
        Self::ensure_admin(&env, &admin);
        if fee < 0 {
            panic!("deployment fee must not be negative");
        }
        env.storage().instance().set(&DataKey::DeploymentFee, &fee);
    }

    /// Return the deployment fee in XLM stroops (zero when never set).
    pub fn get_deployment_fee(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::DeploymentFee)
            .unwrap_or(0)
    }

    /// Deploy a new campaign instance from the stored WASM hash.
    ///
    /// When the deployment fee is greater than zero, `fee` XLM is
    /// transferred from `creator` to the treasury. `creator` must authorize
    /// both the fee transfer and the deployment itself.
    pub fn deploy_campaign(
        env: Env,
        creator: Address,
        xlm_token: Address,
        salt: BytesN<32>,
    ) -> Address {
        creator.require_auth();
        let wasm_hash: BytesN<32> = env
            .storage()
            .instance()
            .get(&DataKey::CampaignWasmHash)
            .expect("wasm hash not set");

        let fee = Self::get_deployment_fee(env.clone());
        if fee > 0 {
            let treasury: Address = env
                .storage()
                .instance()
                .get(&DataKey::Treasury)
                .expect("contract not initialized");
            token::Client::new(&env, &xlm_token).transfer(&creator, &treasury, &fee);
        }

        let contract_address = env
            .deployer()
            .with_address(creator.clone(), salt.clone())
            .deploy(wasm_hash);

        env.events().publish(
            (Symbol::new(&env, "campaign_deployed"),),
            CampaignDeployedEvent {
                creator: creator.clone(),
                contract_address: contract_address.clone(),
                salt,
            },
        );

        contract_address
    }

    /// Return the currently active admin address.
    pub fn get_admin(env: Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("contract not initialized")
    }

    /// Return the treasury address that receives deployment fees.
    pub fn get_treasury(env: Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Treasury)
            .expect("contract not initialized")
    }

    fn ensure_admin(env: &Env, admin: &Address) {
        let stored_admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        if stored_admin != *admin {
            panic!("unauthorized");
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};

    /// Campaign-instance WASM deployed as "v1" code in artifact-based tests.
    mod instance_wasm {
        soroban_sdk::contractimport!(
            file = "../../target/wasm32-unknown-unknown/release/campaign_instance.wasm"
        );
    }

    /// A different WASM (this factory itself) deployed as "v2" code, so the
    /// hash-update retention test can prove existing instances keep their
    /// originally deployed code.
    mod factory_wasm {
        soroban_sdk::contractimport!(
            file = "../../target/wasm32-unknown-unknown/release/campaign_factory.wasm"
        );
    }

    /// Register + initialize a factory; returns `(env, contract_id, admin, treasury)`.
    fn setup() -> (Env, Address, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, CampaignFactoryContract);
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        CampaignFactoryContractClient::new(&env, &contract_id).initialize(&admin, &treasury);
        (env, contract_id, admin, treasury)
    }

    #[test]
    fn initialize_sets_admin_treasury_and_zero_fee() {
        let (env, contract_id, admin, treasury) = setup();
        let client = CampaignFactoryContractClient::new(&env, &contract_id);
        assert_eq!(client.get_admin(), admin);
        assert_eq!(client.get_treasury(), treasury);
        assert_eq!(client.get_deployment_fee(), 0);
        assert!(client.get_wasm_hash().is_none());
    }

    #[test]
    #[should_panic]
    fn double_initialization_panics() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, CampaignFactoryContract);
        let client = CampaignFactoryContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        client.initialize(&admin, &treasury);
        client.initialize(&admin, &treasury);
    }

    #[test]
    fn update_wasm_hash_stores_hash_for_future_deployments() {
        let (env, contract_id, admin, _treasury) = setup();
        let client = CampaignFactoryContractClient::new(&env, &contract_id);
        let hash = BytesN::from_array(&env, &[7u8; 32]);
        assert!(client.get_wasm_hash().is_none());

        client.update_wasm_hash(&admin, &hash);

        assert_eq!(client.get_wasm_hash(), Some(hash));
    }

    #[test]
    #[should_panic]
    fn non_admin_cannot_update_wasm_hash() {
        let (env, contract_id, _admin, _treasury) = setup();
        let client = CampaignFactoryContractClient::new(&env, &contract_id);
        let intruder = Address::generate(&env);
        let hash = BytesN::from_array(&env, &[7u8; 32]);
        client.update_wasm_hash(&intruder, &hash);
    }

    #[test]
    fn set_deployment_fee_updates_fee() {
        let (env, contract_id, admin, _treasury) = setup();
        let client = CampaignFactoryContractClient::new(&env, &contract_id);
        client.set_deployment_fee(&admin, &500_i128);
        assert_eq!(client.get_deployment_fee(), 500);
    }

    #[test]
    #[should_panic]
    fn non_admin_cannot_set_deployment_fee() {
        let (env, contract_id, _admin, _treasury) = setup();
        let client = CampaignFactoryContractClient::new(&env, &contract_id);
        let intruder = Address::generate(&env);
        client.set_deployment_fee(&intruder, &500_i128);
    }

    #[test]
    #[should_panic]
    fn negative_deployment_fee_panics() {
        let (env, contract_id, admin, _treasury) = setup();
        let client = CampaignFactoryContractClient::new(&env, &contract_id);
        client.set_deployment_fee(&admin, &-1_i128);
    }

    #[test]
    #[should_panic]
    fn deploy_campaign_without_wasm_hash_panics() {
        let (env, contract_id, _admin, _treasury) = setup();
        let client = CampaignFactoryContractClient::new(&env, &contract_id);
        let creator = Address::generate(&env);
        let xlm = Address::generate(&env);
        let salt = BytesN::from_array(&env, &[1u8; 32]);
        client.deploy_campaign(&creator, &xlm, &salt);
    }

    #[test]
    fn deploy_campaign_transfers_fee_to_treasury() {
        let (env, contract_id, admin, treasury) = setup();
        let client = CampaignFactoryContractClient::new(&env, &contract_id);
        let wasm_hash = env.deployer().upload_contract_wasm(instance_wasm::WASM);
        client.update_wasm_hash(&admin, &wasm_hash);

        let sac_admin = Address::generate(&env);
        let xlm = env.register_stellar_asset_contract_v2(sac_admin);
        let xlm_client = token::Client::new(&env, &xlm.address());
        let creator = Address::generate(&env);
        token::StellarAssetClient::new(&env, &xlm.address()).mint(&creator, &10_000_i128);

        client.set_deployment_fee(&admin, &250_i128);

        let salt = BytesN::from_array(&env, &[9u8; 32]);
        let campaign = client.deploy_campaign(&creator, &xlm.address(), &salt);

        assert_eq!(xlm_client.balance(&treasury), 250);
        assert_eq!(xlm_client.balance(&creator), 9_750);
        assert_ne!(campaign, contract_id);
    }

    #[test]
    fn deploy_campaign_with_zero_fee_skips_xlm_transfer() {
        let (env, contract_id, admin, _treasury) = setup();
        let client = CampaignFactoryContractClient::new(&env, &contract_id);
        let wasm_hash = env.deployer().upload_contract_wasm(instance_wasm::WASM);
        client.update_wasm_hash(&admin, &wasm_hash);

        let sac_admin = Address::generate(&env);
        let xlm = env.register_stellar_asset_contract_v2(sac_admin);
        let creator = Address::generate(&env);
        // No XLM minted: with the default zero fee no transfer may occur.

        let salt = BytesN::from_array(&env, &[3u8; 32]);
        let campaign = client.deploy_campaign(&creator, &xlm.address(), &salt);

        assert_eq!(client.get_deployment_fee(), 0);
        assert_ne!(campaign, contract_id);
    }

    /// Updating the stored WASM hash must not affect campaign instances that
    /// were already deployed — they keep the code they were deployed with,
    /// while new deployments use the updated hash.
    #[test]
    fn existing_campaigns_retain_original_code_after_hash_update() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|li| {
            li.timestamp = 1_000_000;
        });

        let instance_hash = env.deployer().upload_contract_wasm(instance_wasm::WASM);
        let factory_hash = env.deployer().upload_contract_wasm(factory_wasm::WASM);
        assert_ne!(instance_hash, factory_hash);

        let factory_id = env.register_contract(None, CampaignFactoryContract);
        let factory_client = CampaignFactoryContractClient::new(&env, &factory_id);
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        factory_client.initialize(&admin, &treasury);
        factory_client.update_wasm_hash(&admin, &instance_hash);

        let creator = Address::generate(&env);
        let xlm = Address::generate(&env);

        // v1 deployment from the instance code.
        let salt_a = BytesN::from_array(&env, &[1u8; 32]);
        let campaign_a = factory_client.deploy_campaign(&creator, &xlm, &salt_a);
        let campaign_a_client = instance_wasm::ContractClient::new(&env, &campaign_a);
        campaign_a_client.initialize(
            &creator,
            &1_000_i128,
            &1_086_400_u64,
            &soroban_sdk::Vec::from_array(&env, [Address::generate(&env)]),
        );
        assert_eq!(campaign_a_client.get_goal_amount(), 1_000);

        // Admin rotates the hash to different code.
        factory_client.update_wasm_hash(&admin, &factory_hash);
        assert_eq!(factory_client.get_wasm_hash(), Some(factory_hash));

        // New deployments use the updated code...
        let salt_b = BytesN::from_array(&env, &[2u8; 32]);
        let campaign_b = factory_client.deploy_campaign(&creator, &xlm, &salt_b);
        let campaign_b_client = factory_wasm::ContractClient::new(&env, &campaign_b);
        campaign_b_client.initialize(&Address::generate(&env), &Address::generate(&env));
        assert_eq!(campaign_b_client.get_deployment_fee(), 0);

        // ...while the previously deployed instance is unchanged.
        assert_eq!(campaign_a_client.get_goal_amount(), 1_000);
        assert_eq!(campaign_a_client.get_raised(), 0);
    }
}
