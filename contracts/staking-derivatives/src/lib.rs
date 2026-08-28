#![no_std]

//! Liquid staking derivative accounting engine.
//!
//! Deposits mint internal LSD shares at the current exchange rate. Validator
//! rewards must be funded when reported and accrue only to active shares.
//! Unstaking burns shares immediately and reserves a fixed amount of underlying
//! in a per-user, time-locked request.

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, token, Address, Env, U256,
};

/// Exchange-rate precision: 10_000_000 represents one underlying per share.
pub const RATE_SCALE: i128 = 10_000_000;
const INSTANCE_TTL_THRESHOLD: u32 = 864_000;
const INSTANCE_TTL_BUMP: u32 = 1_728_000;
const PERSISTENT_TTL_THRESHOLD: u32 = 864_000;
const PERSISTENT_TTL_BUMP: u32 = 1_728_000;

#[contract]
pub struct StakingDerivatives;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    InvalidAmount = 3,
    InvalidUnbondingPeriod = 4,
    InsufficientShares = 5,
    RequestNotFound = 6,
    RequestNotReady = 7,
    AlreadyClaimed = 8,
    ArithmeticOverflow = 9,
    ZeroShares = 10,
    NoActiveStake = 11,
    Unauthorized = 12,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnbondingRequest {
    pub amount: i128,
    pub claimed: bool,
    pub owner: Address,
    pub shares_burned: i128,
    pub unlock_timestamp: u64,
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Admin,
    Initialized,
    Underlying,
    UnbondingPeriod,
    TotalActive,
    TotalShares,
    TotalPending,
    TotalRewards,
    NextRequestId,
    ShareBalance(Address),
    Request(u64),
}

#[contractimpl]
impl StakingDerivatives {
    /// Initializes the vault. `unbonding_period` is expressed in seconds.
    pub fn initialize(
        env: Env,
        admin: Address,
        underlying: Address,
        unbonding_period: u64,
    ) -> Result<(), Error> {
        if is_initialized(&env) {
            return Err(Error::AlreadyInitialized);
        }
        if unbonding_period == 0 {
            return Err(Error::InvalidUnbondingPeriod);
        }
        admin.require_auth();

        let storage = env.storage().instance();
        storage.set(&DataKey::Admin, &admin);
        storage.set(&DataKey::Underlying, &underlying);
        storage.set(&DataKey::UnbondingPeriod, &unbonding_period);
        storage.set(&DataKey::TotalActive, &0i128);
        storage.set(&DataKey::TotalShares, &0i128);
        storage.set(&DataKey::TotalPending, &0i128);
        storage.set(&DataKey::TotalRewards, &0i128);
        storage.set(&DataKey::NextRequestId, &0u64);
        storage.set(&DataKey::Initialized, &true);
        bump_instance(&env);
        Ok(())
    }

    /// Deposits underlying and mints proportional LSD shares.
    pub fn deposit(env: Env, user: Address, amount: i128) -> Result<i128, Error> {
        ensure_initialized(&env)?;
        user.require_auth();
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        let active = get_i128(&env, DataKey::TotalActive);
        let supply = get_i128(&env, DataKey::TotalShares);
        let shares = if supply == 0 {
            amount
        } else {
            mul_div(&env, amount, supply, active)?
        };
        if shares == 0 {
            return Err(Error::ZeroShares);
        }

        let new_active = active
            .checked_add(amount)
            .ok_or(Error::ArithmeticOverflow)?;
        let new_supply = supply
            .checked_add(shares)
            .ok_or(Error::ArithmeticOverflow)?;
        let key = DataKey::ShareBalance(user.clone());
        let balance: i128 = env.storage().persistent().get(&key).unwrap_or(0);
        let new_balance = balance
            .checked_add(shares)
            .ok_or(Error::ArithmeticOverflow)?;

        let underlying = get_address(&env, DataKey::Underlying)?;
        token::Client::new(&env, &underlying).transfer(
            &user,
            &env.current_contract_address(),
            &amount,
        );

        env.storage()
            .instance()
            .set(&DataKey::TotalActive, &new_active);
        env.storage()
            .instance()
            .set(&DataKey::TotalShares, &new_supply);
        set_persistent(&env, &key, &new_balance);
        bump_instance(&env);
        env.events()
            .publish((symbol_short!("deposit"), user), (amount, shares));
        Ok(shares)
    }

    /// Funds and accounts validator rewards, increasing the share exchange rate.
    pub fn accrue_rewards(env: Env, admin: Address, amount: i128) -> Result<i128, Error> {
        ensure_admin(&env, &admin)?;
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        let supply = get_i128(&env, DataKey::TotalShares);
        if supply == 0 {
            return Err(Error::NoActiveStake);
        }

        let active = get_i128(&env, DataKey::TotalActive);
        let rewards = get_i128(&env, DataKey::TotalRewards);
        let new_active = active
            .checked_add(amount)
            .ok_or(Error::ArithmeticOverflow)?;
        let new_rewards = rewards
            .checked_add(amount)
            .ok_or(Error::ArithmeticOverflow)?;
        let underlying = get_address(&env, DataKey::Underlying)?;
        token::Client::new(&env, &underlying).transfer(
            &admin,
            &env.current_contract_address(),
            &amount,
        );

        env.storage()
            .instance()
            .set(&DataKey::TotalActive, &new_active);
        env.storage()
            .instance()
            .set(&DataKey::TotalRewards, &new_rewards);
        bump_instance(&env);
        let rate = mul_div(&env, new_active, RATE_SCALE, supply)?;
        env.events()
            .publish((symbol_short!("reward"), admin), (amount, rate));
        Ok(rate)
    }

    /// Burns shares and appends a fixed-value entry to the unbonding queue.
    pub fn request_unstake(env: Env, user: Address, shares: i128) -> Result<u64, Error> {
        ensure_initialized(&env)?;
        user.require_auth();
        if shares <= 0 {
            return Err(Error::InvalidAmount);
        }

        let balance_key = DataKey::ShareBalance(user.clone());
        let balance: i128 = env.storage().persistent().get(&balance_key).unwrap_or(0);
        if shares > balance {
            return Err(Error::InsufficientShares);
        }
        let active = get_i128(&env, DataKey::TotalActive);
        let supply = get_i128(&env, DataKey::TotalShares);
        let amount = if shares == supply {
            active
        } else {
            mul_div(&env, shares, active, supply)?
        };
        if amount == 0 {
            return Err(Error::InvalidAmount);
        }

        let pending = get_i128(&env, DataKey::TotalPending);
        let request_id: u64 = env
            .storage()
            .instance()
            .get(&DataKey::NextRequestId)
            .unwrap_or(0);
        let next_id = request_id.checked_add(1).ok_or(Error::ArithmeticOverflow)?;
        let period: u64 = env
            .storage()
            .instance()
            .get(&DataKey::UnbondingPeriod)
            .ok_or(Error::NotInitialized)?;
        let unlock_timestamp = env
            .ledger()
            .timestamp()
            .checked_add(period)
            .ok_or(Error::ArithmeticOverflow)?;
        let request = UnbondingRequest {
            amount,
            claimed: false,
            owner: user.clone(),
            shares_burned: shares,
            unlock_timestamp,
        };

        env.storage()
            .instance()
            .set(&DataKey::TotalActive, &(active - amount));
        env.storage()
            .instance()
            .set(&DataKey::TotalShares, &(supply - shares));
        env.storage().instance().set(
            &DataKey::TotalPending,
            &pending
                .checked_add(amount)
                .ok_or(Error::ArithmeticOverflow)?,
        );
        env.storage()
            .instance()
            .set(&DataKey::NextRequestId, &next_id);
        set_persistent(&env, &balance_key, &(balance - shares));
        set_persistent(&env, &DataKey::Request(request_id), &request);
        bump_instance(&env);
        env.events().publish(
            (symbol_short!("unstake"), user),
            (request_id, shares, amount, unlock_timestamp),
        );
        Ok(request_id)
    }

    /// Claims a matured unbonding request.
    pub fn claim_unstake(env: Env, user: Address, request_id: u64) -> Result<i128, Error> {
        ensure_initialized(&env)?;
        user.require_auth();
        let key = DataKey::Request(request_id);
        let mut request: UnbondingRequest = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(Error::RequestNotFound)?;
        bump_persistent(&env, &key);
        if request.owner != user {
            return Err(Error::RequestNotFound);
        }
        if request.claimed {
            return Err(Error::AlreadyClaimed);
        }
        if env.ledger().timestamp() < request.unlock_timestamp {
            return Err(Error::RequestNotReady);
        }

        request.claimed = true;
        let pending = get_i128(&env, DataKey::TotalPending);
        env.storage()
            .instance()
            .set(&DataKey::TotalPending, &(pending - request.amount));
        set_persistent(&env, &key, &request);
        bump_instance(&env);

        let underlying = get_address(&env, DataKey::Underlying)?;
        token::Client::new(&env, &underlying).transfer(
            &env.current_contract_address(),
            &user,
            &request.amount,
        );
        env.events()
            .publish((symbol_short!("claim"), user), (request_id, request.amount));
        Ok(request.amount)
    }

    pub fn exchange_rate(env: Env) -> Result<i128, Error> {
        ensure_initialized(&env)?;
        let supply = get_i128(&env, DataKey::TotalShares);
        if supply == 0 {
            return Ok(RATE_SCALE);
        }
        mul_div(
            &env,
            get_i128(&env, DataKey::TotalActive),
            RATE_SCALE,
            supply,
        )
    }

    pub fn convert_to_assets(env: Env, shares: i128) -> Result<i128, Error> {
        ensure_initialized(&env)?;
        if shares < 0 {
            return Err(Error::InvalidAmount);
        }
        let supply = get_i128(&env, DataKey::TotalShares);
        if supply == 0 {
            return Ok(shares);
        }
        mul_div(&env, shares, get_i128(&env, DataKey::TotalActive), supply)
    }

    pub fn convert_to_shares(env: Env, assets: i128) -> Result<i128, Error> {
        ensure_initialized(&env)?;
        if assets < 0 {
            return Err(Error::InvalidAmount);
        }
        let supply = get_i128(&env, DataKey::TotalShares);
        if supply == 0 {
            return Ok(assets);
        }
        mul_div(&env, assets, supply, get_i128(&env, DataKey::TotalActive))
    }

    pub fn share_balance(env: Env, user: Address) -> Result<i128, Error> {
        ensure_initialized(&env)?;
        let key = DataKey::ShareBalance(user);
        let value = env.storage().persistent().get(&key).unwrap_or(0);
        if env.storage().persistent().has(&key) {
            bump_persistent(&env, &key);
        }
        Ok(value)
    }

    pub fn get_request(env: Env, request_id: u64) -> Result<UnbondingRequest, Error> {
        ensure_initialized(&env)?;
        let key = DataKey::Request(request_id);
        let request = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(Error::RequestNotFound)?;
        bump_persistent(&env, &key);
        Ok(request)
    }

    pub fn totals(env: Env) -> Result<(i128, i128, i128, i128), Error> {
        ensure_initialized(&env)?;
        Ok((
            get_i128(&env, DataKey::TotalActive),
            get_i128(&env, DataKey::TotalShares),
            get_i128(&env, DataKey::TotalPending),
            get_i128(&env, DataKey::TotalRewards),
        ))
    }

    pub fn get_unbonding_period(env: Env) -> Result<u64, Error> {
        ensure_initialized(&env)?;
        env.storage()
            .instance()
            .get(&DataKey::UnbondingPeriod)
            .ok_or(Error::NotInitialized)
    }
}

fn ensure_initialized(env: &Env) -> Result<(), Error> {
    if !is_initialized(env) {
        return Err(Error::NotInitialized);
    }
    bump_instance(env);
    Ok(())
}

fn ensure_admin(env: &Env, admin: &Address) -> Result<(), Error> {
    ensure_initialized(env)?;
    let stored = get_address(env, DataKey::Admin)?;
    if &stored != admin {
        return Err(Error::Unauthorized);
    }
    admin.require_auth();
    Ok(())
}

fn is_initialized(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&DataKey::Initialized)
        .unwrap_or(false)
}

fn get_i128(env: &Env, key: DataKey) -> i128 {
    env.storage().instance().get(&key).unwrap_or(0)
}

fn get_address(env: &Env, key: DataKey) -> Result<Address, Error> {
    env.storage()
        .instance()
        .get(&key)
        .ok_or(Error::NotInitialized)
}

fn mul_div(env: &Env, a: i128, b: i128, denominator: i128) -> Result<i128, Error> {
    if a < 0 || b < 0 || denominator <= 0 {
        return Err(Error::ArithmeticOverflow);
    }
    let quotient = U256::from_u128(env, a as u128)
        .mul(&U256::from_u128(env, b as u128))
        .div(&U256::from_u128(env, denominator as u128));
    let value = quotient.to_u128().ok_or(Error::ArithmeticOverflow)?;
    i128::try_from(value).map_err(|_| Error::ArithmeticOverflow)
}

fn set_persistent<T>(env: &Env, key: &DataKey, value: &T)
where
    T: soroban_sdk::IntoVal<Env, soroban_sdk::Val> + Clone,
{
    env.storage().persistent().set(key, value);
    bump_persistent(env, key);
}

fn bump_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL_BUMP);
}

fn bump_persistent(env: &Env, key: &DataKey) {
    env.storage()
        .persistent()
        .extend_ttl(key, PERSISTENT_TTL_THRESHOLD, PERSISTENT_TTL_BUMP);
}

#[cfg(test)]
mod test;
