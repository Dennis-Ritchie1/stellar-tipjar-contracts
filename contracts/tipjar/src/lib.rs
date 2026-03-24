#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, symbol_short, token,
    Address, Env, Map, String, Vec,
};

#[cfg(test)]
extern crate std;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TipWithMessage {
    pub sender: Address,
    pub creator: Address,
    pub amount: i128,
    pub message: String,
    pub metadata: Map<String, String>,
    pub timestamp: u64,
}

/// Storage layout for persistent contract data.
#[derive(Clone)]
#[contracttype]
pub enum DataKey {
    /// Token contract address used for all tips.
    Token,
    /// Creator's currently withdrawable balance held by this contract.
    CreatorBalance(Address),
    /// Historical total tips ever received by creator.
    CreatorTotal(Address),
    /// Emergency pause state (bool).
    Paused,
    /// Contract administrator (Address).
    Admin,
    /// Messages appended for a creator.
    CreatorMessages(Address),
    /// Platform fee in basis points (10_000 == 100%). Max 1_000 (10%).
    PlatformFee,
    /// Address that receives collected platform fees.
    TreasuryAddress,
    /// Cumulative fees collected by the platform.
    TotalFeesCollected,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum TipJarError {
    AlreadyInitialized = 1,
    TokenNotInitialized = 2,
    InvalidAmount = 3,
    NothingToWithdraw = 4,
    MessageTooLong = 5,
    FeeTooHigh = 6,
}

#[contract]
pub struct TipJarContract;

#[contractimpl]
impl TipJarContract {
    /// One-time setup to choose the token contract and administrator for the TipJar.
    pub fn init(env: Env, token: Address, admin: Address) {
        if env.storage().instance().has(&DataKey::Token) {
            panic_with_error!(&env, TipJarError::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Token, &token);
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Paused, &false);
    }

    /// Moves `amount` tokens from `sender` into contract escrow for `creator`.
    ///
    /// If a platform fee is configured, the fee is sent directly to the treasury
    /// and only the remainder is escrowed for the creator.
    pub fn tip(env: Env, sender: Address, creator: Address, amount: i128) {
        Self::require_not_paused(&env);
        if amount <= 0 {
            panic_with_error!(&env, TipJarError::InvalidAmount);
        }

        sender.require_auth();

        let token_id = Self::read_token(&env);
        let token_client = token::Client::new(&env, &token_id);
        let contract_address = env.current_contract_address();

        let fee_bps: u32 = env.storage().instance().get(&DataKey::PlatformFee).unwrap_or(0);
        let fee = Self::calc_fee(amount, fee_bps);
        let creator_amount = amount - fee;

        // Fee goes directly to treasury; creator portion goes into escrow.
        if fee > 0 {
            let treasury: Address = env.storage().instance().get(&DataKey::TreasuryAddress).unwrap();
            token_client.transfer(&sender, &treasury, &fee);

            let total_fees: i128 = env.storage().instance().get(&DataKey::TotalFeesCollected).unwrap_or(0);
            env.storage().instance().set(&DataKey::TotalFeesCollected, &(total_fees + fee));

            env.events()
                .publish((symbol_short!("fee_coll"), treasury), (sender.clone(), fee));
        }

        token_client.transfer(&sender, &contract_address, &creator_amount);

        let creator_balance_key = DataKey::CreatorBalance(creator.clone());
        let creator_total_key = DataKey::CreatorTotal(creator.clone());

        let next_balance: i128 = env.storage().persistent().get(&creator_balance_key).unwrap_or(0) + creator_amount;
        let next_total: i128 = env.storage().persistent().get(&creator_total_key).unwrap_or(0) + creator_amount;

        env.storage().persistent().set(&creator_balance_key, &next_balance);
        env.storage().persistent().set(&creator_total_key, &next_total);

        env.events()
            .publish((symbol_short!("tip"), creator), (sender, creator_amount));
    }

    /// Allows supporters to attach a note and metadata to a tip.
    pub fn tip_with_message(
        env: Env,
        sender: Address,
        creator: Address,
        amount: i128,
        message: String,
        metadata: Map<String, String>,
    ) {
        Self::require_not_paused(&env);
        if amount <= 0 {
            panic_with_error!(&env, TipJarError::InvalidAmount);
        }
        if message.len() > 280 {
            panic_with_error!(&env, TipJarError::MessageTooLong);
        }

        sender.require_auth();

        let token_id = Self::read_token(&env);
        let token_client = token::Client::new(&env, &token_id);
        let contract_address = env.current_contract_address();

        token_client.transfer(&sender, &contract_address, &amount);

        let creator_balance_key = DataKey::CreatorBalance(creator.clone());
        let creator_total_key = DataKey::CreatorTotal(creator.clone());
        let creator_msgs_key = DataKey::CreatorMessages(creator.clone());

        let current_balance: i128 = env.storage().persistent().get(&creator_balance_key).unwrap_or(0);
        let current_total: i128 = env.storage().persistent().get(&creator_total_key).unwrap_or(0);

        env.storage().persistent().set(&creator_balance_key, &(current_balance + amount));
        env.storage().persistent().set(&creator_total_key, &(current_total + amount));

        let timestamp = env.ledger().timestamp();
        let payload = TipWithMessage {
            sender: sender.clone(),
            creator: creator.clone(),
            amount,
            message: message.clone(),
            metadata: metadata.clone(),
            timestamp,
        };
        let mut messages: Vec<TipWithMessage> = env
            .storage()
            .persistent()
            .get(&creator_msgs_key)
            .unwrap_or_else(|| Vec::new(&env));
        messages.push_back(payload);
        env.storage().persistent().set(&creator_msgs_key, &messages);

        env.events().publish(
            (symbol_short!("tip_msg"), creator),
            (sender, amount, message, metadata),
        );
    }

    /// Returns total historical tips for a creator.
    pub fn get_total_tips(env: Env, creator: Address) -> i128 {
        env.storage().persistent().get(&DataKey::CreatorTotal(creator)).unwrap_or(0)
    }

    /// Returns stored messages for a creator.
    pub fn get_messages(env: Env, creator: Address) -> Vec<TipWithMessage> {
        env.storage()
            .persistent()
            .get(&DataKey::CreatorMessages(creator))
            .unwrap_or_else(|| Vec::new(&env))
    }

    /// Returns currently withdrawable escrowed tips for a creator.
    pub fn get_withdrawable_balance(env: Env, creator: Address) -> i128 {
        env.storage().persistent().get(&DataKey::CreatorBalance(creator)).unwrap_or(0)
    }

    /// Allows creator to withdraw their accumulated escrowed tips.
    pub fn withdraw(env: Env, creator: Address) {
        Self::require_not_paused(&env);
        creator.require_auth();

        let key = DataKey::CreatorBalance(creator.clone());
        let amount: i128 = env.storage().persistent().get(&key).unwrap_or(0);
        if amount <= 0 {
            panic_with_error!(&env, TipJarError::NothingToWithdraw);
        }

        let token_client = token::Client::new(&env, &Self::read_token(&env));
        token_client.transfer(&env.current_contract_address(), &creator, &amount);
        env.storage().persistent().set(&key, &0i128);

        env.events().publish((symbol_short!("withdraw"), creator), amount);
    }

    /// Sets the platform fee (in basis points) and treasury address. Admin only.
    /// `fee_bps` must be <= 1_000 (10% max).
    pub fn set_platform_fee(env: Env, admin: Address, fee_bps: u32, treasury: Address) {
        admin.require_auth();
        let stored_admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        if admin != stored_admin {
            panic!("Unauthorized");
        }
        if fee_bps > 1_000 {
            panic_with_error!(&env, TipJarError::FeeTooHigh);
        }
        env.storage().instance().set(&DataKey::PlatformFee, &fee_bps);
        env.storage().instance().set(&DataKey::TreasuryAddress, &treasury);
    }

    /// Returns the total fees collected by the platform.
    pub fn get_total_fees_collected(env: Env) -> i128 {
        env.storage().instance().get(&DataKey::TotalFeesCollected).unwrap_or(0)
    }

    fn calc_fee(amount: i128, fee_bps: u32) -> i128 {
        amount * (fee_bps as i128) / 10_000
    }

    fn read_token(env: &Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Token)
            .unwrap_or_else(|| panic_with_error!(env, TipJarError::TokenNotInitialized))
    }

    /// Emergency pause to stop all state-changing activities (Admin only).
    pub fn pause(env: Env, admin: Address) {
        admin.require_auth();
        let stored_admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        if admin != stored_admin {
            panic!("Unauthorized");
        }
        env.storage().instance().set(&DataKey::Paused, &true);
    }

    /// Resume contract activities after an emergency pause (Admin only).
    pub fn unpause(env: Env, admin: Address) {
        admin.require_auth();
        let stored_admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        if admin != stored_admin {
            panic!("Unauthorized");
        }
        env.storage().instance().set(&DataKey::Paused, &false);
    }

    fn require_not_paused(env: &Env) {
        let is_paused: bool = env.storage().instance().get(&DataKey::Paused).unwrap_or(false);
        if is_paused {
            panic!("Contract is paused");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, token, Address, Env};

    fn setup() -> (Env, Address, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();

        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin.clone())
            .address();

        let admin = Address::generate(&env);
        let contract_id = env.register(TipJarContract, ());
        let tipjar_client = TipJarContractClient::new(&env, &contract_id);
        tipjar_client.init(&token_id, &admin);

        (env, contract_id, token_id, admin)
    }

    #[test]
    fn test_tipping_functionality() {
        let (env, contract_id, token_id, _) = setup();
        let tipjar_client = TipJarContractClient::new(&env, &contract_id);
        let token_client = token::Client::new(&env, &token_id);
        let token_admin_client = token::StellarAssetClient::new(&env, &token_id);
        let sender = Address::generate(&env);
        let creator = Address::generate(&env);

        token_admin_client.mint(&sender, &1_000);
        tipjar_client.tip(&sender, &creator, &250);

        assert_eq!(token_client.balance(&sender), 750);
        assert_eq!(token_client.balance(&contract_id), 250);
        assert_eq!(tipjar_client.get_total_tips(&creator), 250);
    }

    #[test]
    fn test_balance_tracking_and_withdraw() {
        let (env, contract_id, token_id, _) = setup();
        let tipjar_client = TipJarContractClient::new(&env, &contract_id);
        let token_client = token::Client::new(&env, &token_id);
        let token_admin_client = token::StellarAssetClient::new(&env, &token_id);
        let sender_a = Address::generate(&env);
        let sender_b = Address::generate(&env);
        let creator = Address::generate(&env);

        token_admin_client.mint(&sender_a, &1_000);
        token_admin_client.mint(&sender_b, &1_000);

        tipjar_client.tip(&sender_a, &creator, &100);
        tipjar_client.tip(&sender_b, &creator, &300);

        assert_eq!(tipjar_client.get_total_tips(&creator), 400);
        assert_eq!(tipjar_client.get_withdrawable_balance(&creator), 400);

        tipjar_client.withdraw(&creator);

        assert_eq!(tipjar_client.get_withdrawable_balance(&creator), 0);
        assert_eq!(token_client.balance(&creator), 400);
    }

    #[test]
    fn test_platform_fee_split() {
        // 2.5% fee (250 bps) on a 1000-stroop tip: fee=25, creator gets 975.
        let (env, contract_id, token_id, admin) = setup();
        let tipjar_client = TipJarContractClient::new(&env, &contract_id);
        let token_client = token::Client::new(&env, &token_id);
        let token_admin_client = token::StellarAssetClient::new(&env, &token_id);
        let sender = Address::generate(&env);
        let creator = Address::generate(&env);
        let treasury = Address::generate(&env);

        tipjar_client.set_platform_fee(&admin, &250, &treasury);
        token_admin_client.mint(&sender, &1_000);
        tipjar_client.tip(&sender, &creator, &1_000);

        assert_eq!(token_client.balance(&treasury), 25);
        assert_eq!(tipjar_client.get_withdrawable_balance(&creator), 975);
        assert_eq!(tipjar_client.get_total_fees_collected(), 25);
    }

    #[test]
    #[should_panic(expected = "Unauthorized")]
    fn test_set_platform_fee_non_admin_reverts() {
        let (env, contract_id, _, _) = setup();
        let tipjar_client = TipJarContractClient::new(&env, &contract_id);
        let non_admin = Address::generate(&env);
        let treasury = Address::generate(&env);

        tipjar_client.set_platform_fee(&non_admin, &100, &treasury);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #6)")]
    fn test_set_platform_fee_too_high_reverts() {
        let (env, contract_id, _, admin) = setup();
        let tipjar_client = TipJarContractClient::new(&env, &contract_id);
        let treasury = Address::generate(&env);

        tipjar_client.set_platform_fee(&admin, &1_001, &treasury);
    }

    #[test]
    fn test_total_fees_collected_accumulates() {
        let (env, contract_id, token_id, admin) = setup();
        let tipjar_client = TipJarContractClient::new(&env, &contract_id);
        let token_admin_client = token::StellarAssetClient::new(&env, &token_id);
        let sender = Address::generate(&env);
        let creator = Address::generate(&env);
        let treasury = Address::generate(&env);

        tipjar_client.set_platform_fee(&admin, &1_000, &treasury);
        token_admin_client.mint(&sender, &3_000);

        tipjar_client.tip(&sender, &creator, &1_000);
        tipjar_client.tip(&sender, &creator, &1_000);
        tipjar_client.tip(&sender, &creator, &1_000);

        assert_eq!(tipjar_client.get_total_fees_collected(), 300);
        assert_eq!(tipjar_client.get_withdrawable_balance(&creator), 2_700);
    }

    #[test]
    fn test_pause_unpause() {
        let (env, contract_id, _token_id, admin) = setup();
        let tipjar_client = TipJarContractClient::new(&env, &contract_id);

        tipjar_client.pause(&admin);

        let sender = Address::generate(&env);
        let creator = Address::generate(&env);

        let result = tipjar_client.try_tip(&sender, &creator, &100);
        assert!(result.is_err());

        tipjar_client.unpause(&admin);

        let token_admin_client = token::StellarAssetClient::new(&env, &_token_id);
        token_admin_client.mint(&sender, &100);
        tipjar_client.tip(&sender, &creator, &100);
        assert_eq!(tipjar_client.get_total_tips(&creator), 100);
    }

    #[test]
    #[should_panic(expected = "Unauthorized")]
    fn test_pause_admin_only() {
        let (env, contract_id, _, _) = setup();
        let tipjar_client = TipJarContractClient::new(&env, &contract_id);
        let non_admin = Address::generate(&env);

        tipjar_client.pause(&non_admin);
    }

    #[test]
    fn test_withdraw_blocked_when_paused() {
        let (env, contract_id, token_id, admin) = setup();
        let tipjar_client = TipJarContractClient::new(&env, &contract_id);
        let token_admin_client = token::StellarAssetClient::new(&env, &token_id);
        let sender = Address::generate(&env);
        let creator = Address::generate(&env);

        token_admin_client.mint(&sender, &100);
        tipjar_client.tip(&sender, &creator, &100);

        tipjar_client.pause(&admin);

        let result = tipjar_client.try_withdraw(&creator);
        assert!(result.is_err());
    }
}
