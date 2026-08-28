#![no_std]

//! Perpetual futures vAMM mark-price and 8-hour funding-rate engine.

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env, U256,
};

pub const PRICE_SCALE: i128 = 10_000_000;
pub const BPS_SCALE: i128 = 10_000;
pub const FUNDING_INTERVAL: u64 = 8 * 60 * 60;
pub const MAX_FUNDING_RATE_BPS: i128 = 100;
const MAX_INTERVALS_PER_CALL: u64 = 21;
const TTL_THRESHOLD: u32 = 864_000;
const TTL_BUMP: u32 = 1_728_000;

#[contract]
pub struct Perpetuals;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    InvalidPrice = 4,
    InvalidReserve = 5,
    FundingTooEarly = 6,
    ArithmeticOverflow = 7,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketState {
    pub base_reserve: i128,
    pub index_price: i128,
    pub mark_price: i128,
    pub quote_reserve: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FundingState {
    /// Sum of every settled funding rate, in basis points.
    pub cumulative_funding_bps: i128,
    /// Most recently settled 8-hour rate. Positive means longs pay shorts.
    pub current_rate_bps: i128,
    pub last_funding_timestamp: u64,
    pub next_funding_timestamp: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FundingSettlement {
    pub cumulative_funding_bps: i128,
    pub intervals_settled: u64,
    pub rate_bps: i128,
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Admin,
    Oracle,
    Market,
    Funding,
    Initialized,
}

#[contractimpl]
impl Perpetuals {
    /// Initializes the market. The mark price is derived from quote/base reserves.
    pub fn initialize(
        env: Env,
        admin: Address,
        oracle: Address,
        base_reserve: i128,
        quote_reserve: i128,
        index_price: i128,
    ) -> Result<(), Error> {
        if initialized(&env) {
            return Err(Error::AlreadyInitialized);
        }
        if base_reserve <= 0 || quote_reserve <= 0 {
            return Err(Error::InvalidReserve);
        }
        if index_price <= 0 {
            return Err(Error::InvalidPrice);
        }
        admin.require_auth();
        let mark_price = reserve_price(&env, base_reserve, quote_reserve)?;
        let now = env.ledger().timestamp();
        let next = now
            .checked_add(FUNDING_INTERVAL)
            .ok_or(Error::ArithmeticOverflow)?;
        let market = MarketState {
            base_reserve,
            index_price,
            mark_price,
            quote_reserve,
        };
        let funding = FundingState {
            cumulative_funding_bps: 0,
            current_rate_bps: 0,
            last_funding_timestamp: now,
            next_funding_timestamp: next,
        };

        let storage = env.storage().instance();
        storage.set(&DataKey::Admin, &admin);
        storage.set(&DataKey::Oracle, &oracle);
        storage.set(&DataKey::Market, &market);
        storage.set(&DataKey::Funding, &funding);
        storage.set(&DataKey::Initialized, &true);
        bump_ttl(&env);
        env.events()
            .publish((symbol_short!("init"),), (market, funding));
        Ok(())
    }

    /// Updates the vAMM reserves and derives a new mark price from their ratio.
    pub fn update_vamm_reserves(
        env: Env,
        admin: Address,
        base_reserve: i128,
        quote_reserve: i128,
    ) -> Result<i128, Error> {
        require_role(&env, &admin, DataKey::Admin)?;
        if base_reserve <= 0 || quote_reserve <= 0 {
            return Err(Error::InvalidReserve);
        }
        let mut market = market(&env)?;
        market.base_reserve = base_reserve;
        market.quote_reserve = quote_reserve;
        market.mark_price = reserve_price(&env, base_reserve, quote_reserve)?;
        env.storage().instance().set(&DataKey::Market, &market);
        bump_ttl(&env);
        env.events().publish(
            (symbol_short!("vamm_upd"),),
            (base_reserve, quote_reserve, market.mark_price),
        );
        Ok(market.mark_price)
    }

    /// Updates the external index price. Only the configured oracle may call it.
    pub fn update_index_price(env: Env, oracle: Address, index_price: i128) -> Result<(), Error> {
        require_role(&env, &oracle, DataKey::Oracle)?;
        if index_price <= 0 {
            return Err(Error::InvalidPrice);
        }
        let mut market = market(&env)?;
        market.index_price = index_price;
        env.storage().instance().set(&DataKey::Market, &market);
        bump_ttl(&env);
        env.events()
            .publish((symbol_short!("index_upd"),), index_price);
        Ok(())
    }

    /// Returns the rate that would be settled using current mark and index prices.
    pub fn preview_funding_rate(env: Env) -> Result<i128, Error> {
        ensure_initialized(&env)?;
        let market = market(&env)?;
        premium_bps(&env, market.mark_price, market.index_price)
    }

    /// Settles all elapsed 8-hour intervals, bounded per invocation.
    ///
    /// Anyone may call this keeper-friendly operation. If more than 21 periods
    /// elapsed, it can be called repeatedly to catch up without an unbounded call.
    pub fn settle_funding(env: Env) -> Result<FundingSettlement, Error> {
        ensure_initialized(&env)?;
        let now = env.ledger().timestamp();
        let mut funding = funding(&env)?;
        if now < funding.next_funding_timestamp {
            return Err(Error::FundingTooEarly);
        }
        let elapsed = now
            .checked_sub(funding.last_funding_timestamp)
            .ok_or(Error::ArithmeticOverflow)?;
        let intervals = (elapsed / FUNDING_INTERVAL).min(MAX_INTERVALS_PER_CALL);
        let market = market(&env)?;
        let rate = premium_bps(&env, market.mark_price, market.index_price)?;
        let accrued = rate
            .checked_mul(intervals as i128)
            .ok_or(Error::ArithmeticOverflow)?;
        funding.cumulative_funding_bps = funding
            .cumulative_funding_bps
            .checked_add(accrued)
            .ok_or(Error::ArithmeticOverflow)?;
        funding.current_rate_bps = rate;
        let advance = FUNDING_INTERVAL
            .checked_mul(intervals)
            .ok_or(Error::ArithmeticOverflow)?;
        funding.last_funding_timestamp = funding
            .last_funding_timestamp
            .checked_add(advance)
            .ok_or(Error::ArithmeticOverflow)?;
        funding.next_funding_timestamp = funding
            .last_funding_timestamp
            .checked_add(FUNDING_INTERVAL)
            .ok_or(Error::ArithmeticOverflow)?;

        env.storage().instance().set(&DataKey::Funding, &funding);
        bump_ttl(&env);
        let settlement = FundingSettlement {
            cumulative_funding_bps: funding.cumulative_funding_bps,
            intervals_settled: intervals,
            rate_bps: rate,
        };
        env.events()
            .publish((symbol_short!("funding"),), settlement.clone());
        Ok(settlement)
    }

    pub fn get_market(env: Env) -> Result<MarketState, Error> {
        ensure_initialized(&env)?;
        market(&env)
    }

    pub fn get_funding(env: Env) -> Result<FundingState, Error> {
        ensure_initialized(&env)?;
        funding(&env)
    }

    pub fn get_admin(env: Env) -> Result<Address, Error> {
        ensure_initialized(&env)?;
        address(&env, DataKey::Admin)
    }

    pub fn get_oracle(env: Env) -> Result<Address, Error> {
        ensure_initialized(&env)?;
        address(&env, DataKey::Oracle)
    }
}

fn initialized(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&DataKey::Initialized)
        .unwrap_or(false)
}

fn ensure_initialized(env: &Env) -> Result<(), Error> {
    if !initialized(env) {
        return Err(Error::NotInitialized);
    }
    bump_ttl(env);
    Ok(())
}

fn require_role(env: &Env, caller: &Address, key: DataKey) -> Result<(), Error> {
    ensure_initialized(env)?;
    if address(env, key)? != *caller {
        return Err(Error::Unauthorized);
    }
    caller.require_auth();
    Ok(())
}

fn address(env: &Env, key: DataKey) -> Result<Address, Error> {
    env.storage()
        .instance()
        .get(&key)
        .ok_or(Error::NotInitialized)
}

fn market(env: &Env) -> Result<MarketState, Error> {
    env.storage()
        .instance()
        .get(&DataKey::Market)
        .ok_or(Error::NotInitialized)
}

fn funding(env: &Env) -> Result<FundingState, Error> {
    env.storage()
        .instance()
        .get(&DataKey::Funding)
        .ok_or(Error::NotInitialized)
}

fn reserve_price(env: &Env, base: i128, quote: i128) -> Result<i128, Error> {
    mul_div(env, quote, PRICE_SCALE, base)
}

fn premium_bps(env: &Env, mark: i128, index: i128) -> Result<i128, Error> {
    if mark <= 0 || index <= 0 {
        return Err(Error::InvalidPrice);
    }
    let (negative, difference) = if mark >= index {
        (false, mark - index)
    } else {
        (true, index - mark)
    };
    let magnitude = mul_div(env, difference, BPS_SCALE, index)?;
    let signed = if negative { -magnitude } else { magnitude };
    Ok(signed.clamp(-MAX_FUNDING_RATE_BPS, MAX_FUNDING_RATE_BPS))
}

fn mul_div(env: &Env, a: i128, b: i128, denominator: i128) -> Result<i128, Error> {
    if a < 0 || b < 0 || denominator <= 0 {
        return Err(Error::ArithmeticOverflow);
    }
    let result = U256::from_u128(env, a as u128)
        .mul(&U256::from_u128(env, b as u128))
        .div(&U256::from_u128(env, denominator as u128));
    let value = result.to_u128().ok_or(Error::ArithmeticOverflow)?;
    i128::try_from(value).map_err(|_| Error::ArithmeticOverflow)
}

fn bump_ttl(env: &Env) {
    env.storage().instance().extend_ttl(TTL_THRESHOLD, TTL_BUMP);
}

#[cfg(test)]
mod test;
