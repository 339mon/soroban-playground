// Copyright (c) 2026 StellarDevTools
// SPDX-License-Identifier: MIT

//! # Upgradeable Contract  (Issue #587)
//!
//! Enables admins to upgrade the WASM implementation of a contract while
//! preserving on-chain state and address.
//!
//! ## Lifecycle
//! 1. Admin calls `initialize` (once).
//! 2. Admin calls `propose_upgrade` with a new WASM hash; the hash is stored
//!    alongside the current ledger sequence.
//! 3. After `timelock_ledgers` ledgers have elapsed, admin calls
//!    `execute_upgrade` to apply the hash via `env.deployer().update_current_contract_wasm`.
//! 4. Alternatively, admin calls `upgrade_to` (timelock = 0) for an immediate upgrade.
//! 5. Admin may `pause`/`unpause` for emergency halts.

#![no_std]

mod storage;
mod test;
mod types;

use soroban_sdk::{contract, contracterror, contractimpl, symbol_short, Address, BytesN, Env};

use crate::storage::{
    clear_pending_upgrade, get_admin, get_pending_upgrade, get_timelock, is_initialized, is_paused,
    set_admin, set_paused, set_pending_upgrade, set_timelock,
};

/// Errors returned by the upgradeable contract.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    TimelockNotElapsed = 4,
    NoPendingUpgrade = 5,
    ContractPaused = 6,
}

#[contract]
pub struct UpgradeableContract;

#[contractimpl]
impl UpgradeableContract {
    // ── Initialisation ────────────────────────────────────────────────────────

    /// Initialise the contract. Can only be called once.
    /// `timelock_ledgers`: minimum ledgers to wait before executing a proposed upgrade.
    pub fn initialize(
        env: Env,
        admin: Address,
        timelock_ledgers: Option<u32>,
    ) -> Result<(), Error> {
        if is_initialized(&env) {
            return Err(Error::AlreadyInitialized);
        }
        admin.require_auth();
        set_admin(&env, &admin);
        set_timelock(&env, timelock_ledgers.unwrap_or(0));
        env.events().publish((symbol_short!("init"),), admin);
        Ok(())
    }

    // ── Pause / Unpause ───────────────────────────────────────────────────────

    pub fn pause(env: Env, admin: Address) -> Result<(), Error> {
        ensure_initialized(&env)?;
        admin.require_auth();
        require_admin(&env, &admin)?;
        set_paused(&env, true);
        env.events().publish((symbol_short!("paused"),), admin);
        Ok(())
    }

    pub fn unpause(env: Env, admin: Address) -> Result<(), Error> {
        ensure_initialized(&env)?;
        admin.require_auth();
        require_admin(&env, &admin)?;
        set_paused(&env, false);
        env.events().publish((symbol_short!("unpaused"),), admin);
        Ok(())
    }

    // ── Upgrade management ────────────────────────────────────────────────────

    /// Propose a WASM upgrade. Records the hash and current ledger.
    /// If `timelock_ledgers == 0`, the upgrade is applied immediately.
    pub fn propose_upgrade(env: Env, admin: Address, new_hash: BytesN<32>) -> Result<(), Error> {
        ensure_initialized(&env)?;
        not_paused(&env)?;
        admin.require_auth();
        require_admin(&env, &admin)?;

        let timelock = get_timelock(&env);
        if timelock == 0 {
            env.deployer()
                .update_current_contract_wasm(new_hash.clone());
            env.events().publish((symbol_short!("upgraded"),), new_hash);
        } else {
            let current = env.ledger().sequence();
            set_pending_upgrade(&env, &new_hash, current);
            env.events()
                .publish((symbol_short!("proposed"),), (new_hash, current));
        }
        Ok(())
    }

    /// Execute a previously proposed upgrade after the timelock has elapsed.
    pub fn execute_upgrade(env: Env, admin: Address) -> Result<(), Error> {
        ensure_initialized(&env)?;
        not_paused(&env)?;
        admin.require_auth();
        require_admin(&env, &admin)?;

        let (hash, proposed_at) = get_pending_upgrade(&env).ok_or(Error::NoPendingUpgrade)?;
        let timelock = get_timelock(&env);
        let current = env.ledger().sequence();

        if current < proposed_at + timelock {
            return Err(Error::TimelockNotElapsed);
        }

        clear_pending_upgrade(&env);
        env.deployer().update_current_contract_wasm(hash.clone());
        env.events().publish((symbol_short!("upgraded"),), hash);
        Ok(())
    }

    /// Convenience: immediate upgrade (requires timelock == 0).
    pub fn upgrade_to(env: Env, admin: Address, new_hash: BytesN<32>) -> Result<(), Error> {
        Self::propose_upgrade(env, admin, new_hash)
    }

    // ── Read-only ─────────────────────────────────────────────────────────────

    pub fn get_admin(env: Env) -> Result<Address, Error> {
        get_admin(&env)
    }

    pub fn is_initialized(env: Env) -> bool {
        is_initialized(&env)
    }

    pub fn is_paused(env: Env) -> bool {
        is_paused(&env)
    }

    pub fn get_timelock(env: Env) -> Result<u32, Error> {
        ensure_initialized(&env)?;
        Ok(get_timelock(&env))
    }

    pub fn get_pending_upgrade(env: Env) -> Option<(BytesN<32>, u32)> {
        get_pending_upgrade(&env)
    }
}

// ── Private helpers ───────────────────────────────────────────────────────────

fn ensure_initialized(env: &Env) -> Result<(), Error> {
    if !is_initialized(env) {
        return Err(Error::NotInitialized);
    }
    Ok(())
}

fn not_paused(env: &Env) -> Result<(), Error> {
    if is_paused(env) {
        return Err(Error::ContractPaused);
    }
    Ok(())
}

fn require_admin(env: &Env, caller: &Address) -> Result<(), Error> {
    if get_admin(env)? != *caller {
        return Err(Error::Unauthorized);
    }
    Ok(())
}


#![no_std]
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, BytesN, Env,
    Symbol,
};

const TIMELOCK_DELAY_SECONDS: u64 = 172_800; // 48 Hours

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum UpgradeError {
    NotAdmin = 1,
    ContractPaused = 2,
    NoUpgradeProposed = 3,
    TimelockNotExpired = 4,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingUpgrade {
    pub new_wasm_hash: BytesN<32>,
    pub eta: u64,
}

#[contracttype]
pub enum DataKey {
    Admin,
    Paused,
    PendingUpgrade,
}

#[contract]
pub struct UpgradeableContract;

#[contractimpl]
impl UpgradeableContract {
    /// Initialize the contract with an admin authority
    pub fn initialize(env: Env, admin: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic!("Already initialized");
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Paused, &false);
    }

    /// Admin proposes a new WASM hash subject to the 48-hour timelock
    pub fn propose_upgrade(env: Env, new_wasm_hash: BytesN<32>) -> Result<(), UpgradeError> {
        let admin = Self::get_admin(&env)?;
        admin.require_auth();
        Self::ensure_not_paused(&env)?;

        let current_time = env.ledger().timestamp();
        let eta = current_time + TIMELOCK_DELAY_SECONDS;

        let pending = PendingUpgrade {
            new_wasm_hash: new_wasm_hash.clone(),
            eta,
        };

        env.storage()
            .instance()
            .set(&DataKey::PendingUpgrade, &pending);

        env.events().publish(
            (symbol_short!("upgrade"), symbol_short!("proposed")),
            (new_wasm_hash, eta),
        );

        Ok(())
    }

    /// Execute proposed upgrade once timelock delay has elapsed
    pub fn execute_upgrade(env: Env) -> Result<(), UpgradeError> {
        let admin = Self::get_admin(&env)?;
        admin.require_auth();
        Self::ensure_not_paused(&env)?;

        let pending: PendingUpgrade = env
            .storage()
            .instance()
            .get(&DataKey::PendingUpgrade)
            .ok_or(UpgradeError::NoUpgradeProposed)?;

        let current_time = env.ledger().timestamp();
        if current_time < pending.eta {
            return Err(UpgradeError::TimelockNotExpired);
        }

        // Apply WASM code update via Soroban SDK deployer
        env.deployer()
            .update_current_contract_wasm(pending.new_wasm_hash.clone());

        env.storage().instance().remove(&DataKey::PendingUpgrade);

        env.events().publish(
            (symbol_short!("upgrade"), symbol_short!("executed")),
            pending.new_wasm_hash,
        );

        Ok(())
    }

    /// Toggle emergency pause status immediately
    pub fn set_paused(env: Env, paused: bool) -> Result<(), UpgradeError> {
        let admin = Self::get_admin(&env)?;
        admin.require_auth();

        env.storage().instance().set(&DataKey::Paused, &paused);

        env.events().publish(
            (symbol_short!("pause"), Symbol::new(&env, "toggled")),
            paused,
        );

        Ok(())
    }

    /// Helper to fetch admin or fail
    pub fn get_admin(env: &Env) -> Result<Address, UpgradeError> {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(UpgradeError::NotAdmin)
    }

    /// Helper to enforce active state
    fn ensure_not_paused(env: &Env) -> Result<(), UpgradeError> {
        let is_paused: bool = env
            .storage()
            .instance()
            .get(&DataKey::Paused)
            .unwrap_or(false);

        if is_paused {
            return Err(UpgradeError::ContractPaused);
        }
        Ok(())
    }
}