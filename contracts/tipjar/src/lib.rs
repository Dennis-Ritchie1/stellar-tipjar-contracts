#![no_std]
#![deny(unsafe_code)]
#![deny(missing_docs)]

pub mod interfaces;
pub mod integrations;
pub mod security;

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, symbol_short,
    token, Address, BytesN, Env, Map, String, Vec,
};

pub mod upgrade;

#[cfg(test)]
extern crate std;

// Advanced Event System
pub mod events;

// Automated Market Maker
pub mod amm;

// Governance System
pub mod governance;

// Staking and Rewards
pub mod staking;

// Conditional tip execution
pub mod conditions;

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

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchTip {
    pub creator: Address,
    pub token: Address,
    pub amount: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LockedTip {
    pub sender: Address,
    pub creator: Address,
    pub token: Address,
    pub amount: i128,
    pub unlock_timestamp: u64,
}

/// Internal record of a tip for refund tracking.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TipRecord {
    pub id: u64,
    pub sender: Address,
    pub creator: Address,
    pub token: Address,
    pub amount: i128,
    pub timestamp: u64,
    pub refunded: bool,
    pub refund_requested: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TimePeriod {
    AllTime,
    Monthly,
    Weekly,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LeaderboardEntry {
    pub address: Address,
    pub total_amount: i128,
    pub tip_count: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParticipantKind {
    Tipper,
    Creator,
}

/// Query parameters for tip history retrieval.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TipHistoryQuery {
    pub creator: Option<Address>,
    pub sender: Option<Address>,
    pub min_amount: Option<i128>,
    pub max_amount: Option<i128>,
    pub start_time: Option<u64>,
    pub end_time: Option<u64>,
    pub limit: u32,
    pub offset: u32,
}

/// Role enum for role-based access control.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Role {
    Admin,
    Moderator,
    Creator,
}

/// A sponsor-funded tip matching program.
///
/// `match_ratio` is in basis points: 100 = 1:1, 200 = 2:1.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatchingProgram {
    pub id: u64,
    pub sponsor: Address,
    pub creator: Address,
    pub token: Address,
    pub match_ratio: u32,
    pub max_match_amount: i128,
    pub current_matched: i128,
    pub active: bool,
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
    /// Aggregate stats for a tipper in a specific time bucket (bucket_id: 0=AllTime, YYYYMM=Monthly, YYYYWW=Weekly).
    TipperAggregate(Address, u32),
    /// Aggregate stats for a creator in a specific time bucket.
    CreatorAggregate(Address, u32),
    /// Ordered list of all known tipper addresses for a bucket.
    TipperParticipants(u32),
    /// Ordered list of all known creator addresses for a bucket.
    CreatorParticipants(u32),
    /// Locked tip record keyed by (creator, tip_id).
    LockedTip(Address, u64),
    /// Per-creator counter for assigning tip IDs (u64).
    LockedTipCounter(Address),
    /// Global matching program counter.
    MatchingCounter,
    /// Individual matching program by ID.
    MatchingProgram(u64),
    /// Matching program IDs indexed under a creator.
    CreatorMatchingPrograms(Address),
    /// Individual tip record by global tip ID.
    TipRecord(u64),
    /// Global tip counter for assigning tip IDs.
    TipCounter,
    /// Off-chain oracle approval flag keyed by condition ID.
    OffchainCondition(BytesN<32>),
    /// Monotonically increasing contract version, incremented on each upgrade.
    ContractVersion,
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
    BatchTooLarge = 11,
    InsufficientBalance = 12,
    InvalidUnlockTime = 13,
    TipStillLocked = 14,
    LockedTipNotFound = 15,
    MatchingProgramNotFound = 16,
    MatchingProgramInactive = 17,
    InvalidMatchRatio = 18,
    DexNotConfigured = 19,
    NftNotConfigured = 20,
    SwapFailed = 21,
    ConditionFailed = 22,
    /// Caller is not the stored admin; upgrade rejected.
    UpgradeUnauthorized = 23,
}

#[contract]
pub struct TipJarContract;

#[contractimpl]
impl TipJarContract {
    /// One-time setup to choose the administrator for the TipJar.
    pub fn init(env: Env, admin: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic_with_error!(&env, TipJarError::AlreadyInitialized as u32);
        }
        env.storage().instance().put(&DataKey::Admin, &admin);
    }

    /// Sets an off-chain condition flag that can later be referenced in
    /// conditional tip execution.
    pub fn set_offchain_condition(
        env: Env,
        oracle: Address,
        condition_id: BytesN<32>,
        approved: bool,
    ) {
        oracle.require_auth();
        conditions::evaluator::set_offchain_approval(&env, &condition_id, approved);
    }

    /// Adds a token to the whitelist. Admin only.
    pub fn add_token(env: Env, admin: Address, token: Address) {
        admin.require_auth();
        let stored_admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        if admin != stored_admin {
            panic_with_error!(&env, TipJarError::Unauthorized);
        }
        env.storage().instance().set(&DataKey::TokenWhitelist(token), &true);
    }

    /// Transfers `amount` of `token` from `sender` into escrow for `creator`.
    ///
    /// Emits `("tip", creator)` with data `(sender, amount)`.
    pub fn tip(env: Env, sender: Address, creator: Address, token: Address, amount: i128) {
        sender.require_auth();
        if amount <= 0 {
            panic_with_error!(&env, TipJarError::InvalidAmount);
        }
        let whitelisted: bool = env
            .storage()
            .instance()
            .get(&DataKey::TokenWhitelist(token.clone()))
            .unwrap_or(false);
        if !whitelisted {
            panic_with_error!(&env, TipJarError::TokenNotWhitelisted);
        }
        token::Client::new(&env, &token).transfer(&sender, &env.current_contract_address(), &amount);
        let bal_key = DataKey::CreatorBalance(creator.clone(), token.clone());
        let existing_bal: i128 = env.storage().persistent().get(&bal_key)
            .unwrap_or_else(|| env.storage().instance().get(&bal_key).unwrap_or(0));
        let new_bal: i128 = existing_bal + amount;
        env.storage().persistent().set(&bal_key, &new_bal);
        let tot_key = DataKey::CreatorTotal(creator.clone(), token.clone());
        let existing_tot: i128 = env.storage().persistent().get(&tot_key)
            .unwrap_or_else(|| env.storage().instance().get(&tot_key).unwrap_or(0));
        let new_tot: i128 = existing_tot + amount;
        env.storage().persistent().set(&tot_key, &new_tot);
        env.events().publish((symbol_short!("tip"), creator.clone()), (sender, amount));
    }

    /// Withdraws the full escrowed balance for `creator` in `token`.
    ///
    /// Emits `("withdraw", creator)` with data `amount`.
    pub fn withdraw(env: Env, creator: Address, token: Address) {
        creator.require_auth();
        let bal_key = DataKey::CreatorBalance(creator.clone(), token.clone());
        let amount: i128 = env.storage().persistent().get(&bal_key)
            .unwrap_or_else(|| env.storage().instance().get(&bal_key).unwrap_or(0));
        if amount == 0 {
            panic_with_error!(&env, TipJarError::NothingToWithdraw);
        }
        env.storage().persistent().set(&bal_key, &0i128);
        token::Client::new(&env, &token).transfer(&env.current_contract_address(), &creator, &amount);
        env.events().publish((symbol_short!("withdraw"), creator.clone()), amount);
    }

    /// Returns the current withdrawable balance for `creator` in `token`.
    pub fn get_withdrawable_balance(env: Env, creator: Address, token: Address) -> i128 {
        let key = DataKey::CreatorBalance(creator.clone(), token.clone());
        env.storage().persistent().get(&key)
            .unwrap_or_else(|| env.storage().instance().get(&key).unwrap_or(0))
    }

    /// Returns the historical total tips received by `creator` in `token`.
    pub fn get_total_tips(env: Env, creator: Address, token: Address) -> i128 {
        let key = DataKey::CreatorTotal(creator.clone(), token.clone());
        env.storage().persistent().get(&key)
            .unwrap_or_else(|| env.storage().instance().get(&key).unwrap_or(0))
    }

    /// Executes a token tip only if all provided conditions evaluate to true.
    ///
    /// Returns true when the transfer is executed and false when conditions fail.
    pub fn execute_conditional_tip(
        env: Env,
        sender: Address,
        creator: Address,
        token: Address,
        amount: i128,
        condition_list: Vec<conditions::types::Condition>,
    ) -> bool {
        sender.require_auth();

        if amount <= 0 {
            panic_with_error!(&env, TipJarError::InvalidAmount);
        }

        let is_valid = conditions::evaluator::evaluate_all(&env, &condition_list);
        if !is_valid {
            return false;
        }

        let token_client = token::Client::new(&env, &token);
        token_client.transfer(&sender, &creator, &amount);

        env.events().publish(
            (symbol_short!("condtip"), sender.clone()),
            (creator.clone(), token, amount),
        );

        true
    }

    /// Upgrades the contract WASM to the bytecode identified by `new_wasm_hash`.
    ///
    /// Only the stored admin may call this. The version counter is incremented
    /// and an `("upgrade", contract_address)` event is emitted.
    pub fn upgrade(env: Env, admin: Address, new_wasm_hash: BytesN<32>) {
        admin.require_auth();
        let stored_admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        if admin != stored_admin {
            panic_with_error!(&env, TipJarError::UpgradeUnauthorized);
        }
        env.deployer().update_current_contract_wasm(new_wasm_hash);
        let v: u32 = env
            .storage()
            .instance()
            .get(&DataKey::ContractVersion)
            .unwrap_or(1u32);
        env.storage()
            .instance()
            .set(&DataKey::ContractVersion, &(v + 1));
        env.events().publish(
            (symbol_short!("upgrade"), env.current_contract_address()),
            v + 1,
        );
    }

    /// Returns the current contract version (default `1` before any upgrade).
    pub fn version(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::ContractVersion)
            .unwrap_or(1u32)
    }
}