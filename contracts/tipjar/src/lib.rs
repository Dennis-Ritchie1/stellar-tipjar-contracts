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

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Milestone {
    pub id: u64,
    pub creator: Address,
    pub goal_amount: i128,
    pub current_amount: i128,
    pub description: String,
    pub deadline: Option<u64>,
    pub completed: bool,
}

/// Role enum for role-based access control.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Role {
    Admin,
    Moderator,
    Creator,
}

/// Storage layout for persistent contract data.
#[derive(Clone)]
#[contracttype]
pub enum DataKey {
    /// Token contract address whitelist state (bool).
    TokenWhitelist(Address),
    /// Creator's currently withdrawable balance held by this contract per token.
    CreatorBalance(Address, Address), // (creator, token)
    /// Historical total tips ever received by creator per token.
    CreatorTotal(Address, Address),   // (creator, token)
    /// Emergency pause state (bool).
    Paused,
    /// Contract administrator (Address).
    Admin,
    /// Messages appended for a creator.
    CreatorMessages(Address),
    /// Current number of milestones for a creator (used for ID).
    MilestoneCounter(Address),
    /// Data for a specific milestone.
    Milestone(Address, u64),
    /// Active milestone IDs for a creator to track.
    ActiveMilestones(Address),
    /// Maps an address to its assigned Role (persistent).
    UserRole(Address),
    /// Maps a Role to the set of addresses holding it (persistent).
    RoleMembers(Role),
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum TipJarError {
    AlreadyInitialized = 1,
    TokenNotWhitelisted = 2,
    InvalidAmount = 3,
    NothingToWithdraw = 4,
    MessageTooLong = 5,
    MilestoneNotFound = 6,
    MilestoneAlreadyCompleted = 7,
    InvalidGoalAmount = 8,
    Unauthorized = 9,
    RoleNotFound = 10,
}

#[contract]
pub struct TipJarContract;

#[contractimpl]
impl TipJarContract {
    /// One-time setup to choose the administrator for the TipJar.
    pub fn init(env: Env, admin: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic_with_error!(&env, TipJarError::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Paused, &false);
        grant_role_internal(&env, &admin, &admin, Role::Admin);
    }

    /// Adds a token to the whitelist (Admin only).
    pub fn add_token(env: Env, admin: Address, token: Address) {
        admin.require_auth();
        require_role(&env, &admin, Role::Admin);
        env.storage()
            .instance()
            .set(&DataKey::TokenWhitelist(token), &true);
    }

    /// Removes a token from the whitelist (Admin only).
    pub fn remove_token(env: Env, admin: Address, token: Address) {
        admin.require_auth();
        require_role(&env, &admin, Role::Admin);
        env.storage()
            .instance()
            .set(&DataKey::TokenWhitelist(token), &false);
    }

    /// Moves `amount` tokens from `sender` into contract escrow for `creator`.
    ///
    /// The sender must authorize this call and have enough token balance.
    pub fn tip(env: Env, sender: Address, creator: Address, token: Address, amount: i128) {
        if Self::is_paused(&env) {
            panic!("Contract is paused");
        }
        if amount <= 0 {
            panic_with_error!(&env, TipJarError::InvalidAmount);
        }

        if !Self::is_whitelisted(env.clone(), token.clone()) {
            panic_with_error!(&env, TipJarError::TokenNotWhitelisted);
        }

        sender.require_auth();

        let token_client = token::Client::new(&env, &token);
        let contract_address = env.current_contract_address();

        token_client.transfer(&sender, &contract_address, &amount);

        let creator_balance_key = DataKey::CreatorBalance(creator.clone(), token.clone());
        let creator_total_key = DataKey::CreatorTotal(creator.clone(), token.clone());

        let next_balance: i128 = env.storage().persistent().get(&creator_balance_key).unwrap_or(0) + amount;
        let next_total: i128 = env.storage().persistent().get(&creator_total_key).unwrap_or(0) + amount;

        env.storage().persistent().set(&creator_balance_key, &next_balance);
        env.storage().persistent().set(&creator_total_key, &next_total);

        // Event topics: ("tip", creator, token). Event data: (sender, amount).
        env.events()
            .publish((symbol_short!("tip"), creator, token), (sender, amount));
    }

    /// Allows supporters to attach a note and metadata to a tip.
    pub fn tip_with_message(
        env: Env,
        sender: Address,
        creator: Address,
        token: Address,
        amount: i128,
        message: String,
        metadata: Map<String, String>,
    ) {
        if Self::is_paused(&env) {
            panic!("Contract is paused");
        }
        if amount <= 0 {
            panic_with_error!(&env, TipJarError::InvalidAmount);
        }
        if message.len() > 280 {
            panic_with_error!(&env, TipJarError::MessageTooLong);
        }
        if !Self::is_whitelisted(env.clone(), token.clone()) {
            panic_with_error!(&env, TipJarError::TokenNotWhitelisted);
        }

        sender.require_auth();

        let token_client = token::Client::new(&env, &token);
        let contract_address = env.current_contract_address();

        token_client.transfer(&sender, &contract_address, &amount);

        let balance_key = DataKey::CreatorBalance(creator.clone(), token.clone());
        let total_key = DataKey::CreatorTotal(creator.clone(), token.clone());
        let msgs_key = DataKey::CreatorMessages(creator.clone());

        let current_balance: i128 = env.storage().persistent().get(&balance_key).unwrap_or(0);
        let current_total: i128 = env.storage().persistent().get(&total_key).unwrap_or(0);

        env.storage().persistent().set(&balance_key, &(current_balance + amount));
        env.storage().persistent().set(&total_key, &(current_total + amount));

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
            .get(&msgs_key)
            .unwrap_or_else(|| Vec::new(&env));
        messages.push_back(payload);
        env.storage().persistent().set(&msgs_key, &messages);

        env.events().publish(
            (symbol_short!("tip_msg"), creator.clone()),
            (sender, amount, message, metadata),
        );
    }

    /// Returns total historical tips for a creator for a specific token.
    pub fn get_total_tips(env: Env, creator: Address, token: Address) -> i128 {
        env.storage().persistent().get(&DataKey::CreatorTotal(creator, token)).unwrap_or(0)
    }

    /// Returns stored messages for a creator.
    pub fn get_messages(env: Env, creator: Address) -> Vec<TipWithMessage> {
        env.storage()
            .persistent()
            .get(&DataKey::CreatorMessages(creator))
            .unwrap_or_else(|| Vec::new(&env))
    }

    /// Returns currently withdrawable escrowed tips for a creator for a specific token.
    pub fn get_withdrawable_balance(env: Env, creator: Address, token: Address) -> i128 {
        env.storage().persistent().get(&DataKey::CreatorBalance(creator, token)).unwrap_or(0)
    }

    /// Allows creator to withdraw their accumulated escrowed tips for a specific token.
    pub fn withdraw(env: Env, creator: Address, token: Address) {
        if Self::is_paused(&env) {
            panic!("Contract is paused");
        }
        creator.require_auth();
        require_role(&env, &creator, Role::Creator);

        let key = DataKey::CreatorBalance(creator.clone(), token.clone());
        let amount: i128 = env.storage().persistent().get(&key).unwrap_or(0);
        if amount <= 0 {
            panic_with_error!(&env, TipJarError::NothingToWithdraw);
        }

        let token_client = token::Client::new(&env, &token);
        let contract_address = env.current_contract_address();

        token_client.transfer(&contract_address, &creator, &amount);
        env.storage().persistent().set(&key, &0i128);

        env.events()
            .publish((symbol_short!("withdraw"), creator, token), amount);
    }

    pub fn is_whitelisted(env: Env, token: Address) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::TokenWhitelist(token))
            .unwrap_or(false)
    }

    /// Emergency pause to stop all state-changing activities (Admin or Moderator only).
    pub fn pause(env: Env, admin: Address) {
        admin.require_auth();
        require_any_role(&env, &admin, &[Role::Admin, Role::Moderator]);
        env.storage().instance().set(&DataKey::Paused, &true);
    }

    /// Resume contract activities after an emergency pause (Admin or Moderator only).
    pub fn unpause(env: Env, admin: Address) {
        admin.require_auth();
        require_any_role(&env, &admin, &[Role::Admin, Role::Moderator]);
        env.storage().instance().set(&DataKey::Paused, &false);
    }

    /// Internal helper to check if the contract is paused.
    fn is_paused(env: &Env) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::Paused)
            .unwrap_or(false)
    }

    /// Returns `true` iff `target` currently holds `role`. No authorization required.
    pub fn has_role(env: Env, target: Address, role: Role) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::UserRole(target))
            .map(|r: Role| r == role)
            .unwrap_or(false)
    }

    /// Grants `role` to `target`. Caller must be Admin.
    pub fn grant_role(env: Env, caller: Address, target: Address, role: Role) {
        caller.require_auth();
        require_role(&env, &caller, Role::Admin);
        grant_role_internal(&env, &caller, &target, role);
    }

    /// Revokes the role from `target`. Caller must be Admin.
    /// Panics with `RoleNotFound` if `target` holds no role.
    pub fn revoke_role(env: Env, caller: Address, target: Address) {
        caller.require_auth();
        require_role(&env, &caller, Role::Admin);

        // Read the current role; panic if absent.
        let role: Role = env
            .storage()
            .persistent()
            .get(&DataKey::UserRole(target.clone()))
            .unwrap_or_else(|| panic_with_error!(&env, TipJarError::RoleNotFound));

        // Remove the UserRole entry.
        env.storage()
            .persistent()
            .remove(&DataKey::UserRole(target.clone()));

        // Remove target from RoleMembers.
        let members_key = DataKey::RoleMembers(role.clone());
        let mut members: Vec<Address> = env
            .storage()
            .persistent()
            .get(&members_key)
            .unwrap_or_else(|| Vec::new(&env));
        let mut filtered: Vec<Address> = Vec::new(&env);
        for a in members.iter() {
            if a != target {
                filtered.push_back(a);
            }
        }
        members = filtered;
        env.storage().persistent().set(&members_key, &members);

        // Emit role_rvk event: topics = (symbol, target, role), data = caller.
        env.events().publish(
            (symbol_short!("role_rvk"), target.clone(), role.clone()),
            caller.clone(),
        );
    }
}

/// Shared write path for granting a role. Used by `grant_role` and `init`.
///
/// - Writes `role` to `DataKey::UserRole(target)` in persistent storage.
/// - Adds `target` to `DataKey::RoleMembers(role)` (deduplicating if already present).
/// - Emits a `role_grant` event.
fn grant_role_internal(env: &Env, caller: &Address, target: &Address, role: Role) {
    // Write the user → role mapping.
    env.storage()
        .persistent()
        .set(&DataKey::UserRole(target.clone()), &role);

    // Read-modify-write the role → members list.
    let members_key = DataKey::RoleMembers(role.clone());
    let mut members: Vec<Address> = env
        .storage()
        .persistent()
        .get(&members_key)
        .unwrap_or_else(|| Vec::new(env));

    // Only add if not already present (dedup).
    let already_present = members.iter().any(|a| a == *target);
    if !already_present {
        members.push_back(target.clone());
    }
    env.storage().persistent().set(&members_key, &members);

    // Emit role_grnt event: topics = (symbol, target, role), data = caller.
    env.events().publish(
        (symbol_short!("role_grnt"), target.clone(), role),
        caller.clone(),
    );
}

/// Panics with `TipJarError::Unauthorized` unless `addr` currently holds `required`.
fn require_role(env: &Env, addr: &Address, required: Role) {
    // Inline the has_role logic (has_role public fn is implemented in task 3.1).
    let holds: bool = env
        .storage()
        .persistent()
        .get(&DataKey::UserRole(addr.clone()))
        .map(|r: Role| r == required)
        .unwrap_or(false);

    if !holds {
        panic_with_error!(env, TipJarError::Unauthorized);
    }
}

/// Panics with `TipJarError::Unauthorized` unless `addr` holds at least one role in `roles`.
fn require_any_role(env: &Env, addr: &Address, roles: &[Role]) {
    let assigned: Option<Role> = env
        .storage()
        .persistent()
        .get(&DataKey::UserRole(addr.clone()));

    let has_any = match assigned {
        Some(r) => roles.iter().any(|required| *required == r),
        None => false,
    };

    if !has_any {
        panic_with_error!(env, TipJarError::Unauthorized);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, token, Address, Env};

    /// Returns (env, contract_id, token_id_1, token_id_2, admin).
    fn setup() -> (Env, Address, Address, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();

        let token_admin = Address::generate(&env);
        let token_id_1 = env
            .register_stellar_asset_contract_v2(token_admin.clone())
            .address();
        let token_id_2 = env
            .register_stellar_asset_contract_v2(token_admin.clone())
            .address();

        let admin = Address::generate(&env);
        let contract_id = env.register(TipJarContract, ());
        let tipjar_client = TipJarContractClient::new(&env, &contract_id);
        tipjar_client.init(&admin);
        tipjar_client.add_token(&admin, &token_id_1);

        (env, contract_id, token_id_1, token_id_2, admin)
    }

    #[test]
    fn test_tipping_functionality_multi_token() {
        let (env, contract_id, token_id_1, token_id_2, admin) = setup();
        let tipjar_client = TipJarContractClient::new(&env, &contract_id);
        let token_client_1 = token::Client::new(&env, &token_id_1);
        let token_client_2 = token::Client::new(&env, &token_id_2);
        let token_admin_client_1 = token::StellarAssetClient::new(&env, &token_id_1);
        let token_admin_client_2 = token::StellarAssetClient::new(&env, &token_id_2);

        let sender = Address::generate(&env);
        let creator = Address::generate(&env);

        token_admin_client_1.mint(&sender, &1_000);
        token_admin_client_2.mint(&sender, &1_000);

        // Success for whitelisted token 1
        tipjar_client.tip(&sender, &creator, &token_id_1, &250);
        assert_eq!(token_client_1.balance(&sender), 750);
        assert_eq!(token_client_1.balance(&contract_id), 250);
        assert_eq!(tipjar_client.get_total_tips(&creator, &token_id_1), 250);

        // Failure for non-whitelisted token 2
        let result = tipjar_client.try_tip(&sender, &creator, &token_id_2, &100);
        assert!(result.is_err());

        // Success after whitelisting token 2
        tipjar_client.add_token(&admin, &token_id_2);
        tipjar_client.tip(&sender, &creator, &token_id_2, &300);
        assert_eq!(token_client_2.balance(&sender), 700);
        assert_eq!(tipjar_client.get_total_tips(&creator, &token_id_2), 300);
    }

    #[test]
    fn test_balance_tracking_and_withdraw() {
        let (env, contract_id, token_id, _, admin) = setup();
        let tipjar_client = TipJarContractClient::new(&env, &contract_id);
        let token_client = token::Client::new(&env, &token_id);
        let token_admin_client = token::StellarAssetClient::new(&env, &token_id);
        let sender_a = Address::generate(&env);
        let sender_b = Address::generate(&env);
        let creator = Address::generate(&env);

        token_admin_client.mint(&sender_a, &1_000);
        token_admin_client.mint(&sender_b, &1_000);

        tipjar_client.tip(&sender_a, &creator, &token_id, &100);
        tipjar_client.tip(&sender_b, &creator, &token_id, &300);

        assert_eq!(tipjar_client.get_total_tips(&creator, &token_id), 400);
        assert_eq!(tipjar_client.get_withdrawable_balance(&creator, &token_id), 400);

        // Grant Creator role so withdraw passes role check
        tipjar_client.grant_role(&admin, &creator, &Role::Creator);
        tipjar_client.withdraw(&creator, &token_id);

        assert_eq!(tipjar_client.get_withdrawable_balance(&creator, &token_id), 0);
        assert_eq!(token_client.balance(&creator), 400);
    }

    #[test]
    fn test_multi_token_balance_and_withdraw() {
        let (env, contract_id, token_id_1, token_id_2, admin) = setup();
        let tipjar_client = TipJarContractClient::new(&env, &contract_id);
        let token_client_1 = token::Client::new(&env, &token_id_1);
        let token_client_2 = token::Client::new(&env, &token_id_2);
        let token_admin_client_1 = token::StellarAssetClient::new(&env, &token_id_1);
        let token_admin_client_2 = token::StellarAssetClient::new(&env, &token_id_2);

        let sender = Address::generate(&env);
        let creator = Address::generate(&env);

        tipjar_client.add_token(&admin, &token_id_2);
        token_admin_client_1.mint(&sender, &1_000);
        token_admin_client_2.mint(&sender, &1_000);

        tipjar_client.tip(&sender, &creator, &token_id_1, &100);
        tipjar_client.tip(&sender, &creator, &token_id_2, &300);

        assert_eq!(tipjar_client.get_withdrawable_balance(&creator, &token_id_1), 100);
        assert_eq!(tipjar_client.get_withdrawable_balance(&creator, &token_id_2), 300);

        // Grant Creator role so withdraw passes role check
        tipjar_client.grant_role(&admin, &creator, &Role::Creator);

        // Withdraw token 1
        tipjar_client.withdraw(&creator, &token_id_1);
        assert_eq!(tipjar_client.get_withdrawable_balance(&creator, &token_id_1), 0);
        assert_eq!(token_client_1.balance(&creator), 100);
        assert_eq!(tipjar_client.get_withdrawable_balance(&creator, &token_id_2), 300);

        // Withdraw token 2
        tipjar_client.withdraw(&creator, &token_id_2);
        assert_eq!(tipjar_client.get_withdrawable_balance(&creator, &token_id_2), 0);
        assert_eq!(token_client_2.balance(&creator), 300);
    }

    #[test]
    #[should_panic]
    fn test_invalid_tip_amount() {
        let (env, contract_id, token_id_1, _, _) = setup();
        let tipjar_client = TipJarContractClient::new(&env, &contract_id);
        let token_admin_client = token::StellarAssetClient::new(&env, &token_id_1);
        let sender = Address::generate(&env);
        let creator = Address::generate(&env);

        token_admin_client.mint(&sender, &100);
        tipjar_client.tip(&sender, &creator, &token_id_1, &0);
    }

    #[test]
    fn test_pause_unpause() {
        let (env, contract_id, token_id_1, _, admin) = setup();
        let tipjar_client = TipJarContractClient::new(&env, &contract_id);

        tipjar_client.pause(&admin);

        let sender = Address::generate(&env);
        let creator = Address::generate(&env);

        let result = tipjar_client.try_tip(&sender, &creator, &token_id_1, &100);
        assert!(result.is_err());

        tipjar_client.unpause(&admin);

        let token_admin_client = token::StellarAssetClient::new(&env, &token_id_1);
        token_admin_client.mint(&sender, &100);
        tipjar_client.tip(&sender, &creator, &token_id_1, &100);
        assert_eq!(tipjar_client.get_total_tips(&creator, &token_id_1), 100);
    }

    #[test]
    #[should_panic]
    fn test_pause_admin_only() {
        let (env, contract_id, _, _, _) = setup();
        let tipjar_client = TipJarContractClient::new(&env, &contract_id);
        let non_admin = Address::generate(&env);

        tipjar_client.pause(&non_admin);
    }

    // ── Task 6.1: grant_role / has_role happy paths ──────────────────────────

    #[test]
    fn test_grant_role_admin_variant() {
        let (env, contract_id, _, _, admin) = setup();
        let client = TipJarContractClient::new(&env, &contract_id);
        let target = Address::generate(&env);

        client.grant_role(&admin, &target, &Role::Admin);
        assert!(client.has_role(&target, &Role::Admin));
    }

    #[test]
    fn test_grant_role_moderator_variant() {
        let (env, contract_id, _, _, admin) = setup();
        let client = TipJarContractClient::new(&env, &contract_id);
        let target = Address::generate(&env);

        client.grant_role(&admin, &target, &Role::Moderator);
        assert!(client.has_role(&target, &Role::Moderator));
    }

    #[test]
    fn test_grant_role_creator_variant() {
        let (env, contract_id, _, _, admin) = setup();
        let client = TipJarContractClient::new(&env, &contract_id);
        let target = Address::generate(&env);

        client.grant_role(&admin, &target, &Role::Creator);
        assert!(client.has_role(&target, &Role::Creator));
    }

    #[test]
    fn test_grant_role_idempotent_no_duplicate_in_role_members() {
        let (env, contract_id, _, _, admin) = setup();
        let client = TipJarContractClient::new(&env, &contract_id);
        let target = Address::generate(&env);

        // Grant the same role twice.
        client.grant_role(&admin, &target, &Role::Moderator);
        client.grant_role(&admin, &target, &Role::Moderator);

        // has_role must still be true.
        assert!(client.has_role(&target, &Role::Moderator));

        // RoleMembers must not contain a duplicate entry.
        let members: Vec<Address> = env
            .as_contract(&contract_id, || {
                env.storage()
                    .persistent()
                    .get(&DataKey::RoleMembers(Role::Moderator))
                    .unwrap_or_else(|| Vec::new(&env))
            });
        let count = members.iter().filter(|a| *a == target).count();
        assert_eq!(count, 1, "target should appear exactly once in RoleMembers");
    }

    // ── Task 6.2: revoke_role happy path and error cases ─────────────────────

    #[test]
    fn test_revoke_role_removes_role() {
        let (env, contract_id, _, _, admin) = setup();
        let client = TipJarContractClient::new(&env, &contract_id);
        let target = Address::generate(&env);

        client.grant_role(&admin, &target, &Role::Moderator);
        assert!(client.has_role(&target, &Role::Moderator));

        client.revoke_role(&admin, &target);
        assert!(!client.has_role(&target, &Role::Moderator));
    }

    #[test]
    fn test_revoke_role_unassigned_returns_role_not_found() {
        let (env, contract_id, _, _, admin) = setup();
        let client = TipJarContractClient::new(&env, &contract_id);
        let target = Address::generate(&env);

        let result = client.try_revoke_role(&admin, &target);
        assert_eq!(
            result.unwrap_err().unwrap(),
            TipJarError::RoleNotFound.into()
        );
    }

    #[test]
    fn test_non_admin_grant_role_returns_unauthorized() {
        let (env, contract_id, _, _, _) = setup();
        let client = TipJarContractClient::new(&env, &contract_id);
        let non_admin = Address::generate(&env);
        let target = Address::generate(&env);

        let result = client.try_grant_role(&non_admin, &target, &Role::Creator);
        assert_eq!(
            result.unwrap_err().unwrap(),
            TipJarError::Unauthorized.into()
        );
    }

    #[test]
    fn test_non_admin_revoke_role_returns_unauthorized() {
        let (env, contract_id, _, _, admin) = setup();
        let client = TipJarContractClient::new(&env, &contract_id);
        let non_admin = Address::generate(&env);
        let target = Address::generate(&env);

        // Give target a role so the revoke would otherwise succeed.
        client.grant_role(&admin, &target, &Role::Creator);

        let result = client.try_revoke_role(&non_admin, &target);
        assert_eq!(
            result.unwrap_err().unwrap(),
            TipJarError::Unauthorized.into()
        );
    }

    // ── Task 6.3: enforced existing functions ────────────────────────────────

    #[test]
    fn test_moderator_can_pause_and_unpause() {
        let (env, contract_id, _, _, admin) = setup();
        let client = TipJarContractClient::new(&env, &contract_id);
        let moderator = Address::generate(&env);

        client.grant_role(&admin, &moderator, &Role::Moderator);

        // Should succeed without panic.
        client.pause(&moderator);
        client.unpause(&moderator);
    }

    #[test]
    fn test_creator_cannot_pause() {
        let (env, contract_id, _, _, admin) = setup();
        let client = TipJarContractClient::new(&env, &contract_id);
        let creator = Address::generate(&env);

        client.grant_role(&admin, &creator, &Role::Creator);

        let result = client.try_pause(&creator);
        assert_eq!(
            result.unwrap_err().unwrap(),
            TipJarError::Unauthorized.into()
        );
    }

    #[test]
    fn test_moderator_cannot_add_token() {
        let (env, contract_id, _, token_id_2, admin) = setup();
        let client = TipJarContractClient::new(&env, &contract_id);
        let moderator = Address::generate(&env);

        client.grant_role(&admin, &moderator, &Role::Moderator);

        let result = client.try_add_token(&moderator, &token_id_2);
        assert_eq!(
            result.unwrap_err().unwrap(),
            TipJarError::Unauthorized.into()
        );
    }

    #[test]
    fn test_non_creator_cannot_withdraw() {
        let (env, contract_id, token_id_1, _, admin) = setup();
        let client = TipJarContractClient::new(&env, &contract_id);
        let moderator = Address::generate(&env);

        client.grant_role(&admin, &moderator, &Role::Moderator);

        // Moderator has no Creator role — withdraw must be rejected.
        let result = client.try_withdraw(&moderator, &token_id_1);
        assert_eq!(
            result.unwrap_err().unwrap(),
            TipJarError::Unauthorized.into()
        );
    }

    #[test]
    fn test_init_auto_grants_admin_role() {
        // setup() already calls init(); verify the admin address has Admin role.
        let (env, contract_id, _, _, admin) = setup();
        let client = TipJarContractClient::new(&env, &contract_id);

        assert!(client.has_role(&admin, &Role::Admin));
    }
}
