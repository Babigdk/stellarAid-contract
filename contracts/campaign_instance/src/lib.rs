#![no_std]

#[cfg(test)]
extern crate std;

use soroban_sdk::{contract, contractimpl, contracttype, token, Address, Env, Symbol, Vec};

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    Initialized,
    GoalAmount,
    EndTime,
    AcceptedAssets,
    Raised,
}

#[contracttype]
#[derive(Clone)]
pub struct DonationReceivedEvent {
    pub donor: Address,
    pub token: Address,
    pub amount: i128,
}

#[contract]
pub struct CampaignInstanceContract;

#[contractimpl]
impl CampaignInstanceContract {
    /// Initialize a campaign instance deployed by the campaign factory.
    ///
    /// * `goal_amount` must be positive.
    /// * `end_time` must be in the future.
    /// * `accepted_assets` must contain at least one token address.
    pub fn initialize(
        env: Env,
        admin: Address,
        goal_amount: i128,
        end_time: u64,
        accepted_assets: Vec<Address>,
    ) {
        admin.require_auth();
        if env.storage().instance().has(&DataKey::Initialized) {
            panic!("already initialized");
        }
        if goal_amount <= 0 {
            panic!("goal_amount must be positive");
        }
        if end_time <= env.ledger().timestamp() {
            panic!("end_time must be in the future");
        }
        if accepted_assets.is_empty() {
            panic!("accepted_assets must not be empty");
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::GoalAmount, &goal_amount);
        env.storage().instance().set(&DataKey::EndTime, &end_time);
        env.storage()
            .instance()
            .set(&DataKey::AcceptedAssets, &accepted_assets);
        env.storage().instance().set(&DataKey::Raised, &0_i128);
        env.storage().instance().set(&DataKey::Initialized, &true);
        shared::version::seed(&env, env!("CARGO_PKG_VERSION"));
    }

    shared::impl_semver_queries!();

    pub fn get_admin(env: Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("contract not initialized")
    }

    pub fn get_goal_amount(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::GoalAmount)
            .unwrap_or(0)
    }

    pub fn get_end_time(env: Env) -> u64 {
        env.storage().instance().get(&DataKey::EndTime).unwrap_or(0)
    }

    pub fn get_accepted_assets(env: Env) -> Vec<Address> {
        env.storage()
            .instance()
            .get(&DataKey::AcceptedAssets)
            .unwrap_or(Vec::new(&env))
    }

    pub fn get_raised(env: Env) -> i128 {
        env.storage().instance().get(&DataKey::Raised).unwrap_or(0)
    }

    /// Donate `amount` of `token` to this campaign. The token must be one of
    /// the campaign's accepted assets and the campaign must not have ended.
    pub fn donate(env: Env, donor: Address, token: Address, amount: i128) {
        donor.require_auth();
        if amount <= 0 {
            panic!("amount must be positive");
        }
        let end_time: u64 = env
            .storage()
            .instance()
            .get(&DataKey::EndTime)
            .expect("contract not initialized");
        if env.ledger().timestamp() > end_time {
            panic!("campaign has ended");
        }
        let accepted_assets: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::AcceptedAssets)
            .expect("contract not initialized");
        if !accepted_assets.contains(&token) {
            panic!("token not in accepted assets");
        }
        token::Client::new(&env, &token).transfer(
            &donor,
            &env.current_contract_address(),
            &amount,
        );
        let raised: i128 = env.storage().instance().get(&DataKey::Raised).unwrap_or(0);
        env.storage().instance().set(&DataKey::Raised, &(raised + amount));
        env.events().publish(
            (Symbol::new(&env, "donation_received"),),
            DonationReceivedEvent { donor, token, amount },
        );
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};

    const NOW: u64 = 1_000_000;

    fn setup_env() -> Env {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|li| {
            li.timestamp = NOW;
        });
        env
    }

    fn deploy_mock_token(env: &Env, admin: &Address) -> Address {
        let token_id = env.register_contract(None, MockToken);
        MockTokenClient::new(env, &token_id).initialize(admin);
        token_id
    }

    fn deploy_instance<'a>(
        env: &'a Env,
        goal: i128,
        end_time: u64,
        accepted_assets: Vec<Address>,
    ) -> (Address, CampaignInstanceContractClient<'a>) {
        let contract_id = env.register_contract(None, CampaignInstanceContract);
        let client = CampaignInstanceContractClient::new(env, &contract_id);
        let admin = Address::generate(env);
        client.initialize(&admin, &goal, &end_time, &accepted_assets);
        (contract_id, client)
    }

    #[test]
    fn initialize_succeeds_with_valid_args() {
        let env = setup_env();
        let contract_id = env.register_contract(None, CampaignInstanceContract);
        let client = CampaignInstanceContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let token = Address::generate(&env);
        let goal = 5_000_i128;
        let end_time = NOW + 86_400;
        let assets = Vec::from_array(&env, [token.clone()]);

        client.initialize(&admin, &goal, &end_time, &assets);

        assert_eq!(client.get_admin(), admin);
        assert_eq!(client.get_goal_amount(), goal);
        assert_eq!(client.get_end_time(), end_time);
        assert_eq!(client.get_accepted_assets(), assets);
        assert_eq!(client.get_raised(), 0);
    }

    #[test]
    #[should_panic]
    fn double_initialization_panics() {
        let env = setup_env();
        let contract_id = env.register_contract(None, CampaignInstanceContract);
        let client = CampaignInstanceContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let token = Address::generate(&env);
        let assets = Vec::from_array(&env, [token]);
        client.initialize(&admin, &1_000_i128, &(NOW + 1_000), &assets);
        client.initialize(&admin, &2_000_i128, &(NOW + 2_000), &assets);
    }

    #[test]
    #[should_panic]
    fn zero_goal_amount_panics() {
        let env = setup_env();
        let contract_id = env.register_contract(None, CampaignInstanceContract);
        let client = CampaignInstanceContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let token = Address::generate(&env);
        client.initialize(
            &admin,
            &0_i128,
            &(NOW + 1_000),
            &Vec::from_array(&env, [token]),
        );
    }

    #[test]
    #[should_panic]
    fn negative_goal_amount_panics() {
        let env = setup_env();
        let contract_id = env.register_contract(None, CampaignInstanceContract);
        let client = CampaignInstanceContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let token = Address::generate(&env);
        client.initialize(
            &admin,
            &-100_i128,
            &(NOW + 1_000),
            &Vec::from_array(&env, [token]),
        );
    }

    #[test]
    #[should_panic]
    fn end_time_in_the_past_panics() {
        let env = setup_env();
        let contract_id = env.register_contract(None, CampaignInstanceContract);
        let client = CampaignInstanceContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let token = Address::generate(&env);
        client.initialize(&admin, &1_000_i128, &NOW, &Vec::from_array(&env, [token]));
    }

    #[test]
    #[should_panic]
    fn end_time_strictly_in_the_past_panics() {
        let env = setup_env();
        let contract_id = env.register_contract(None, CampaignInstanceContract);
        let client = CampaignInstanceContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let token = Address::generate(&env);
        client.initialize(&admin, &1_000_i128, &(NOW - 100), &Vec::from_array(&env, [token]));
    }

    #[test]
    #[should_panic]
    fn empty_accepted_assets_panics() {
        let env = setup_env();
        let contract_id = env.register_contract(None, CampaignInstanceContract);
        let client = CampaignInstanceContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        client.initialize(&admin, &1_000_i128, &(NOW + 1_000), &Vec::new(&env));
    }

    #[test]
    fn donate_accepts_mock_token_and_tracks_raised() {
        let env = setup_env();
        let token_admin = Address::generate(&env);
        let donor = Address::generate(&env);
        let token_id = deploy_mock_token(&env, &token_admin);
        let (contract_id, client) = deploy_instance(
            &env,
            5_000_i128,
            NOW + 86_400,
            Vec::from_array(&env, [token_id.clone()]),
        );

        let token_client = MockTokenClient::new(&env, &token_id);
        token_client.mint(&donor, &2_000_i128);
        assert_eq!(token_client.balance(&donor), 2_000_i128);

        client.donate(&donor, &token_id, &750_i128);

        assert_eq!(client.get_raised(), 750_i128);
        assert_eq!(token_client.balance(&contract_id), 750_i128);
        assert_eq!(token_client.balance(&donor), 1_250_i128);
    }

    #[test]
    #[should_panic]
    fn donate_rejects_token_not_in_accepted_assets() {
        let env = setup_env();
        let token_admin = Address::generate(&env);
        let donor = Address::generate(&env);
        let accepted_id = deploy_mock_token(&env, &token_admin);
        let other_id = deploy_mock_token(&env, &token_admin);
        let (_, client) = deploy_instance(
            &env,
            5_000_i128,
            NOW + 86_400,
            Vec::from_array(&env, [accepted_id]),
        );

        client.donate(&donor, &other_id, &100_i128);
    }

    #[test]
    #[should_panic]
    fn donate_after_end_time_panics() {
        let env = setup_env();
        let token_admin = Address::generate(&env);
        let donor = Address::generate(&env);
        let token_id = deploy_mock_token(&env, &token_admin);
        let (_, client) = deploy_instance(
            &env,
            5_000_i128,
            NOW + 10,
            Vec::from_array(&env, [token_id.clone()]),
        );
        MockTokenClient::new(&env, &token_id).mint(&donor, &100_i128);

        env.ledger().with_mut(|li| {
            li.timestamp = NOW + 11;
        });
        client.donate(&donor, &token_id, &100_i128);
    }

    #[test]
    #[should_panic]
    fn donate_zero_amount_panics() {
        let env = setup_env();
        let token_admin = Address::generate(&env);
        let donor = Address::generate(&env);
        let token_id = deploy_mock_token(&env, &token_admin);
        let (_, client) = deploy_instance(
            &env,
            5_000_i128,
            NOW + 86_400,
            Vec::from_array(&env, [token_id.clone()]),
        );

        client.donate(&donor, &token_id, &0_i128);
    }

    #[contracttype]
    #[derive(Clone)]
    enum MockTokenKey {
        Admin,
        Balance(Address),
    }

    /// Minimal SEP-41-style mock token used to exercise donation flows in
    /// tests without a real Stellar asset contract.
    #[contract]
    pub struct MockToken;

    #[contractimpl]
    impl MockToken {
        pub fn initialize(env: Env, admin: Address) {
            admin.require_auth();
            if env.storage().instance().has(&MockTokenKey::Admin) {
                panic!("already initialized");
            }
            env.storage().instance().set(&MockTokenKey::Admin, &admin);
        }

        pub fn mint(env: Env, to: Address, amount: i128) {
            let admin: Address = env
                .storage()
                .instance()
                .get(&MockTokenKey::Admin)
                .expect("not initialized");
            admin.require_auth();
            let balance = Self::balance(env.clone(), to.clone()) + amount;
            env.storage()
                .persistent()
                .set(&MockTokenKey::Balance(to), &balance);
        }

        pub fn balance(env: Env, id: Address) -> i128 {
            env.storage()
                .persistent()
                .get(&MockTokenKey::Balance(id))
                .unwrap_or(0)
        }

        pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
            from.require_auth();
            if amount <= 0 {
                panic!("amount must be positive");
            }
            let from_balance = Self::balance(env.clone(), from.clone());
            if from_balance < amount {
                panic!("insufficient balance");
            }
            let to_balance = Self::balance(env.clone(), to.clone());
            env.storage()
                .persistent()
                .set(&MockTokenKey::Balance(from), &(from_balance - amount));
            env.storage()
                .persistent()
                .set(&MockTokenKey::Balance(to), &(to_balance + amount));
        }
    }
}
