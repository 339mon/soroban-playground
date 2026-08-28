// Copyright (c) 2026 StellarDevTools
// SPDX-License-Identifier: MIT

//! # Automated Market Maker (AMM) — Dynamic Fee Pool with Volatility-Adjusted Slippage Curve
//!
//! Implements x * y = k with:
//! - LP token minting/burning
//! - Dynamic swap fee adjusted by recent price volatility and pool utilization
//! - Volatility-adjusted slippage curve (higher volatility → steeper price impact)
//! - TWAP price accumulator updated on every swap
//! - Minimum liquidity (1000 units) locked permanently
//! - Volatility oracle: tracks short-window price variance
//! - Utilization ratio: fee scales with reserve depletion depth

#![no_std]

mod storage;
mod test;
mod types;

use soroban_sdk::{contract, contractimpl, symbol_short, Address, Env};

use crate::storage::{
    get_collection_stats, get_fee_bps, get_floor_price, get_last_ts, get_lp, get_nft_collection,
    get_price_a_cum, get_price_b_cum, get_reserve_a, get_reserve_b, get_token_a, get_token_b,
    get_total_fees, get_total_lp, get_total_volume, get_vol_ema, get_vol_window_sum,
    get_vol_window_count, is_initialized, set_admin, set_collection_stats, set_fee_bps,
    set_floor_price, set_last_ts, set_lp, set_nft_collection, set_price_a_cum, set_price_b_cum,
    set_reserve_a, set_reserve_b, set_token_a, set_token_b, set_total_fees, set_total_lp,
    set_total_volume, set_vol_ema, set_vol_window_sum, set_vol_window_count,
};
use crate::types::{CollectionStats, Error, PoolConfig};

/// Minimum liquidity permanently locked on first deposit.
const MIN_LIQUIDITY: i128 = 1_000;
/// Precision multiplier for TWAP accumulators.
const TWAP_PRECISION: i128 = 1_000_000;
/// Base fee in basis points (0.30%).
const BASE_FEE_BPS: i128 = 30;
/// Maximum fee in basis points (3.00%).
const MAX_FEE_BPS: i128 = 300;
/// Minimum fee in basis points (0.05%).
const MIN_FEE_BPS: i128 = 5;
/// EMA smoothing factor numerator (alpha = 1/8 ≈ 0.125).
const EMA_ALPHA_NUM: i128 = 1;
const EMA_ALPHA_DEN: i128 = 8;
/// Volatility window size for rolling variance.
const VOL_WINDOW: i128 = 10;
/// Utilization fee uplift per 10% utilization above 50% (in bps).
const UTIL_UPLIFT_BPS: i128 = 5;
/// Slippage curve steepness multiplier at high volatility (scaled by 1000).
const SLIPPAGE_CURVE_BASE: i128 = 1_000;

#[contract]
pub struct AmmPool;

#[contractimpl]
impl AmmPool {
    // ── Initialisation ────────────────────────────────────────────────────────

    /// Create the pool for `token_a` / `token_b` with an optional custom base fee.
    pub fn initialize(
        env: Env,
        admin: Address,
        token_a: Address,
        token_b: Address,
        fee_bps: Option<i128>,
    ) -> Result<(), Error> {
        if is_initialized(&env) {
            return Err(Error::AlreadyInitialized);
        }
        admin.require_auth();
        set_admin(&env, &admin);
        set_token_a(&env, &token_a);
        set_token_b(&env, &token_b);
        let base_fee = fee_bps.unwrap_or(BASE_FEE_BPS);
        if base_fee < MIN_FEE_BPS || base_fee > MAX_FEE_BPS {
            return Err(Error::InvalidFee);
        }
        set_fee_bps(&env, base_fee);
        set_last_ts(&env, env.ledger().timestamp());
        set_vol_ema(&env, 0);
        set_vol_window_sum(&env, 0);
        set_vol_window_count(&env, 0);
        Ok(())
    }

    /// Initialize NFT AMM pool with collection tracking.
    pub fn initialize_nft(
        env: Env,
        admin: Address,
        token_a: Address,
        token_b: Address,
        nft_collection: Address,
        fee_bps: Option<i128>,
    ) -> Result<(), Error> {
        if !is_initialized(&env) {
            Self::initialize(env.clone(), admin, token_a, token_b, fee_bps)?;
        }
        set_nft_collection(&env, nft_collection.clone());

        let stats = CollectionStats {
            floor_price: 0,
            ceiling_price: 0,
            total_volume: 0,
            trade_count: 0,
            unique_holders: 0,
            last_update: env.ledger().timestamp(),
        };
        set_collection_stats(&env, &stats);

        env.events()
            .publish((symbol_short!("nft_init"),), nft_collection);
        Ok(())
    }

    // ── Liquidity management ──────────────────────────────────────────────────

    /// Deposit `amount_a` of token_a and `amount_b` of token_b.
    /// Returns LP tokens minted.
    pub fn add_liquidity(
        env: Env,
        provider: Address,
        amount_a: i128,
        amount_b: i128,
        min_lp: i128,
    ) -> Result<i128, Error> {
        ensure_initialized(&env)?;
        provider.require_auth();
        if amount_a <= 0 || amount_b <= 0 {
            return Err(Error::ZeroAmount);
        }

        let ra = get_reserve_a(&env);
        let rb = get_reserve_b(&env);
        let total_lp = get_total_lp(&env);

        let lp_minted = if total_lp == 0 {
            let lp = isqrt(amount_a, amount_b)?;
            if lp <= MIN_LIQUIDITY {
                return Err(Error::InsufficientLiquidity);
            }
            set_total_lp(&env, MIN_LIQUIDITY);
            lp - MIN_LIQUIDITY
        } else {
            let lp_a = amount_a.checked_mul(total_lp).ok_or(Error::Overflow)? / ra;
            let lp_b = amount_b.checked_mul(total_lp).ok_or(Error::Overflow)? / rb;
            lp_a.min(lp_b)
        };

        if lp_minted < min_lp {
            return Err(Error::SlippageExceeded);
        }

        set_reserve_a(&env, ra + amount_a);
        set_reserve_b(&env, rb + amount_b);
        let new_total = get_total_lp(&env) + lp_minted;
        set_total_lp(&env, new_total);
        set_lp(&env, &provider, get_lp(&env, &provider) + lp_minted);

        env.events().publish((symbol_short!("add_liq"),), lp_minted);
        Ok(lp_minted)
    }

    /// Burn `lp_amount` LP tokens and return (amount_a, amount_b).
    pub fn remove_liquidity(
        env: Env,
        provider: Address,
        lp_amount: i128,
        min_a: i128,
        min_b: i128,
    ) -> Result<(i128, i128), Error> {
        ensure_initialized(&env)?;
        provider.require_auth();
        if lp_amount <= 0 {
            return Err(Error::ZeroAmount);
        }

        let lp_bal = get_lp(&env, &provider);
        if lp_bal < lp_amount {
            return Err(Error::InsufficientLpBalance);
        }

        let total_lp = get_total_lp(&env);
        let ra = get_reserve_a(&env);
        let rb = get_reserve_b(&env);

        let out_a = lp_amount.checked_mul(ra).ok_or(Error::Overflow)? / total_lp;
        let out_b = lp_amount.checked_mul(rb).ok_or(Error::Overflow)? / total_lp;

        if out_a < min_a || out_b < min_b {
            return Err(Error::SlippageExceeded);
        }

        set_lp(&env, &provider, lp_bal - lp_amount);
        set_total_lp(&env, total_lp - lp_amount);
        set_reserve_a(&env, ra - out_a);
        set_reserve_b(&env, rb - out_b);

        env.events().publish((symbol_short!("rm_liq"),), lp_amount);
        Ok((out_a, out_b))
    }

    // ── Swap ──────────────────────────────────────────────────────────────────

    /// Swap `amount_in` of `token_in` for the other token.
    ///
    /// Fee is computed dynamically as:
    ///   fee = clamp(BASE_FEE + volatility_uplift + utilization_uplift, MIN_FEE, MAX_FEE)
    ///
    /// Slippage curve steepness scales with current EMA volatility:
    ///   effective_k = k * (1 + vol_multiplier)  — larger vol makes price impact steeper.
    ///
    /// `min_out` enforces slippage protection. Returns the output amount.
    pub fn swap(
        env: Env,
        trader: Address,
        token_in: Address,
        amount_in: i128,
        min_out: i128,
    ) -> Result<i128, Error> {
        ensure_initialized(&env)?;
        trader.require_auth();
        if amount_in <= 0 {
            return Err(Error::ZeroAmount);
        }

        let (ra, rb, a_to_b) = reserves_for_token_in(&env, &token_in)?;
        if ra == 0 || rb == 0 {
            return Err(Error::InsufficientLiquidity);
        }

        // Compute dynamic fee based on volatility + utilization.
        let dynamic_fee = compute_dynamic_fee(&env, amount_in, ra, rb)?;

        // Compute output using volatility-adjusted slippage curve.
        let amount_out = get_amount_out_dynamic(amount_in, ra, rb, dynamic_fee, &env)?;

        if amount_out < min_out {
            return Err(Error::SlippageExceeded);
        }
        if amount_out == 0 {
            return Err(Error::ZeroOutput);
        }

        // Update reserves.
        let (new_ra, new_rb) = if a_to_b {
            (ra + amount_in, rb - amount_out)
        } else {
            (rb - amount_out, ra + amount_in)
        };
        set_reserve_a(&env, if a_to_b { new_ra } else { new_rb });
        set_reserve_b(&env, if a_to_b { new_rb } else { new_ra });

        // Update TWAP accumulators.
        update_twap(&env, ra, rb);

        // Update volatility oracle with new price observation.
        update_volatility_oracle(&env, ra, rb, amount_in, amount_out)?;

        // Track volume and fees.
        record_swap_metrics(&env, amount_in, dynamic_fee)?;

        env.events().publish((symbol_short!("swap"),), amount_out);
        Ok(amount_out)
    }

    // ── Read-only ─────────────────────────────────────────────────────────────

    /// Preview output for a swap without state changes, using current dynamic fee.
    pub fn get_amount_out(env: Env, amount_in: i128, token_in: Address) -> Result<i128, Error> {
        ensure_initialized(&env)?;
        let (ra, rb, _) = reserves_for_token_in(&env, &token_in)?;
        let dynamic_fee = compute_dynamic_fee(&env, amount_in, ra, rb)?;
        get_amount_out_dynamic(amount_in, ra, rb, dynamic_fee, &env)
    }

    /// Returns the current dynamic fee that would be applied to a swap.
    pub fn get_current_fee(env: Env, amount_in: i128, token_in: Address) -> Result<i128, Error> {
        ensure_initialized(&env)?;
        let (ra, rb, _) = reserves_for_token_in(&env, &token_in)?;
        compute_dynamic_fee(&env, amount_in, ra, rb)
    }

    pub fn get_reserves(env: Env) -> Result<(i128, i128), Error> {
        ensure_initialized(&env)?;
        Ok((get_reserve_a(&env), get_reserve_b(&env)))
    }

    pub fn get_lp_balance(env: Env, addr: Address) -> Result<i128, Error> {
        ensure_initialized(&env)?;
        Ok(get_lp(&env, &addr))
    }

    pub fn get_total_lp(env: Env) -> Result<i128, Error> {
        ensure_initialized(&env)?;
        Ok(get_total_lp(&env))
    }

    /// Returns (price_a_cumulative, price_b_cumulative, last_timestamp).
    pub fn get_twap(env: Env) -> Result<(i128, i128, u64), Error> {
        ensure_initialized(&env)?;
        Ok((
            get_price_a_cum(&env),
            get_price_b_cum(&env),
            get_last_ts(&env),
        ))
    }

    pub fn get_fee_bps(env: Env) -> Result<i128, Error> {
        ensure_initialized(&env)?;
        Ok(get_fee_bps(&env))
    }

    /// Returns current volatility EMA (scaled by TWAP_PRECISION).
    pub fn get_volatility(env: Env) -> Result<i128, Error> {
        ensure_initialized(&env)?;
        Ok(get_vol_ema(&env))
    }

    /// Returns full pool configuration including dynamic fee parameters.
    pub fn get_pool_config(env: Env) -> Result<PoolConfig, Error> {
        ensure_initialized(&env)?;
        Ok(PoolConfig {
            base_fee_bps: get_fee_bps(&env),
            min_fee_bps: MIN_FEE_BPS,
            max_fee_bps: MAX_FEE_BPS,
            vol_ema: get_vol_ema(&env),
            reserve_a: get_reserve_a(&env),
            reserve_b: get_reserve_b(&env),
        })
    }

    // ── NFT Collection Analytics ──────────────────────────────────────────────

    pub fn get_collection_stats(env: Env) -> Result<CollectionStats, Error> {
        ensure_initialized(&env)?;
        get_collection_stats(&env).ok_or(Error::NotInitialized)
    }

    pub fn update_floor_price(env: Env, admin: Address, new_floor: i128) -> Result<(), Error> {
        ensure_initialized(&env)?;
        admin.require_auth();
        if new_floor < 0 {
            return Err(Error::ZeroAmount);
        }
        set_floor_price(&env, new_floor);
        if let Some(mut stats) = get_collection_stats(&env) {
            stats.floor_price = new_floor;
            stats.last_update = env.ledger().timestamp();
            set_collection_stats(&env, &stats);
        }
        env.events()
            .publish((symbol_short!("floor_upd"),), new_floor);
        Ok(())
    }

    pub fn get_pool_metrics(env: Env) -> Result<(i128, i128), Error> {
        ensure_initialized(&env)?;
        Ok((get_total_volume(&env), get_total_fees(&env)))
    }

    pub fn get_floor_price(env: Env) -> Result<i128, Error> {
        ensure_initialized(&env)?;
        Ok(get_floor_price(&env))
    }

    pub fn get_nft_collection(env: Env) -> Result<Option<Address>, Error> {
        ensure_initialized(&env)?;
        Ok(get_nft_collection(&env))
    }
}

// ── Dynamic fee engine ────────────────────────────────────────────────────────

/// Compute dynamic fee = BASE_FEE + vol_uplift + util_uplift, clamped to [MIN, MAX].
///
/// vol_uplift:  proportional to current EMA volatility.
///              At 1% price movement EMA → +30 bps uplift (doubles base fee).
///
/// util_uplift: every 10 pp of utilization above 50% adds UTIL_UPLIFT_BPS.
///              Utilization = amount_in / reserve_in (capped at 100%).
fn compute_dynamic_fee(env: &Env, amount_in: i128, ra: i128, _rb: i128) -> Result<i128, Error> {
    let base = get_fee_bps(env);

    // Volatility uplift: vol_ema is scaled by TWAP_PRECISION.
    // At vol_ema = 10_000 (1% move) → uplift = 30 bps.
    let vol_ema = get_vol_ema(env);
    let vol_uplift = vol_ema
        .checked_mul(30)
        .ok_or(Error::Overflow)?
        .checked_div(10_000)
        .unwrap_or(0);

    // Utilization uplift: uplift per 10% utilization above 50%.
    let utilization_bps = if ra > 0 {
        amount_in
            .checked_mul(10_000)
            .ok_or(Error::Overflow)?
            .checked_div(ra)
            .unwrap_or(10_000)
            .min(10_000)
    } else {
        0
    };
    let util_above_50 = (utilization_bps - 5_000).max(0); // bps above 50%
    let util_uplift = util_above_50
        .checked_mul(UTIL_UPLIFT_BPS)
        .ok_or(Error::Overflow)?
        / 1_000; // per 10% = per 1000 bps

    let dynamic_fee = base + vol_uplift + util_uplift;
    Ok(dynamic_fee.clamp(MIN_FEE_BPS, MAX_FEE_BPS))
}

/// Volatility-adjusted constant-product output.
///
/// The slippage curve steepens under high volatility by applying a multiplier
/// to the effective input reserve, making price impact larger:
///
///   vol_multiplier = 1 + (vol_ema * SLIPPAGE_CURVE_BASE) / TWAP_PRECISION / 1000
///   effective_ra   = ra * vol_multiplier
///   amount_out     = (amount_in_after_fee * rb) / (effective_ra + amount_in_after_fee)
fn get_amount_out_dynamic(
    amount_in: i128,
    ra: i128,
    rb: i128,
    fee_bps: i128,
    env: &Env,
) -> Result<i128, Error> {
    if ra == 0 || rb == 0 {
        return Err(Error::InsufficientLiquidity);
    }
    let fee_factor = 10_000 - fee_bps;
    let amount_in_after_fee = amount_in
        .checked_mul(fee_factor)
        .ok_or(Error::Overflow)?
        .checked_div(10_000)
        .ok_or(Error::Overflow)?;

    // Volatility curve steepness: higher vol → larger effective_ra → more slippage.
    let vol_ema = get_vol_ema(env);
    // vol_mult = 1000 + (vol_ema * SLIPPAGE_CURVE_BASE) / TWAP_PRECISION
    // effective_ra = ra * vol_mult / 1000
    let vol_mult_scaled = SLIPPAGE_CURVE_BASE
        + vol_ema
            .checked_mul(SLIPPAGE_CURVE_BASE)
            .ok_or(Error::Overflow)?
            / TWAP_PRECISION;
    let effective_ra = ra
        .checked_mul(vol_mult_scaled)
        .ok_or(Error::Overflow)?
        / SLIPPAGE_CURVE_BASE;

    let numerator = amount_in_after_fee
        .checked_mul(rb)
        .ok_or(Error::Overflow)?;
    let denominator = effective_ra
        .checked_add(amount_in_after_fee)
        .ok_or(Error::Overflow)?;
    if denominator == 0 {
        return Err(Error::InsufficientLiquidity);
    }
    Ok(numerator / denominator)
}

// ── Volatility oracle ─────────────────────────────────────────────────────────

/// Update the volatility EMA with the latest price observation.
///
/// Price return = |price_after - price_before| / price_before, scaled by TWAP_PRECISION.
/// EMA update:   vol_ema = vol_ema * (1 - alpha) + return_sq * alpha
/// where alpha = EMA_ALPHA_NUM / EMA_ALPHA_DEN.
fn update_volatility_oracle(
    env: &Env,
    ra_before: i128,
    rb_before: i128,
    _amount_in: i128,
    _amount_out: i128,
) -> Result<(), Error> {
    if ra_before == 0 || rb_before == 0 {
        return Ok(());
    }
    let ra_after = get_reserve_a(env);
    let rb_after = get_reserve_b(env);
    if ra_after == 0 || rb_after == 0 {
        return Ok(());
    }

    // Spot price before and after (scaled by TWAP_PRECISION).
    let price_before = rb_before
        .checked_mul(TWAP_PRECISION)
        .ok_or(Error::Overflow)?
        / ra_before;
    let price_after = rb_after
        .checked_mul(TWAP_PRECISION)
        .ok_or(Error::Overflow)?
        / ra_after;

    // Absolute return in TWAP_PRECISION units.
    let price_diff = if price_after > price_before {
        price_after - price_before
    } else {
        price_before - price_after
    };

    // Rolling window sum for variance estimation.
    let win_sum = get_vol_window_sum(env);
    let win_count = get_vol_window_count(env);
    let new_sum = win_sum
        .saturating_add(price_diff)
        .saturating_sub(if win_count >= VOL_WINDOW {
            // Remove oldest approximation (treat as average when full).
            win_sum / win_count.max(1)
        } else {
            0
        });
    let new_count = win_count.min(VOL_WINDOW - 1) + 1;
    set_vol_window_sum(env, new_sum);
    set_vol_window_count(env, new_count);

    // Update EMA: new_ema = old_ema * (1 - alpha) + new_obs * alpha
    let rolling_avg = new_sum / new_count.max(1);
    let old_ema = get_vol_ema(env);
    let new_ema = old_ema
        .checked_mul(EMA_ALPHA_DEN - EMA_ALPHA_NUM)
        .ok_or(Error::Overflow)?
        / EMA_ALPHA_DEN
        + rolling_avg
            .checked_mul(EMA_ALPHA_NUM)
            .ok_or(Error::Overflow)?
            / EMA_ALPHA_DEN;
    set_vol_ema(env, new_ema);
    Ok(())
}

// ── Private helpers ───────────────────────────────────────────────────────────

fn ensure_initialized(env: &Env) -> Result<(), Error> {
    if !is_initialized(env) {
        return Err(Error::NotInitialized);
    }
    Ok(())
}

fn reserves_for_token_in(env: &Env, token_in: &Address) -> Result<(i128, i128, bool), Error> {
    let token_a = get_token_a(env)?;
    let token_b = get_token_b(env)?;
    if token_in == &token_a {
        Ok((get_reserve_a(env), get_reserve_b(env), true))
    } else if token_in == &token_b {
        Ok((get_reserve_b(env), get_reserve_a(env), false))
    } else {
        Err(Error::InvalidToken)
    }
}

fn record_swap_metrics(env: &Env, amount_in: i128, fee_bps: i128) -> Result<(), Error> {
    let fee_amount = amount_in
        .checked_mul(fee_bps)
        .ok_or(Error::Overflow)?
        / 10_000;
    set_total_volume(env, get_total_volume(env) + amount_in);
    set_total_fees(env, get_total_fees(env) + fee_amount);
    Ok(())
}

/// Integer square root of a * b (Babylonian method).
fn isqrt(a: i128, b: i128) -> Result<i128, Error> {
    let product = a.checked_mul(b).ok_or(Error::Overflow)?;
    if product == 0 {
        return Ok(0);
    }
    let mut x = product;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + product / x) / 2;
    }
    Ok(x)
}

/// Update TWAP price accumulators using time elapsed since last swap.
fn update_twap(env: &Env, ra: i128, rb: i128) {
    let now = env.ledger().timestamp();
    let last = get_last_ts(env);
    if now <= last || ra == 0 || rb == 0 {
        set_last_ts(env, now);
        return;
    }
    let elapsed = (now - last) as i128;
    let price_a = rb.saturating_mul(TWAP_PRECISION) / ra;
    let price_b = ra.saturating_mul(TWAP_PRECISION) / rb;
    set_price_a_cum(
        env,
        get_price_a_cum(env).saturating_add(price_a.saturating_mul(elapsed)),
    );
    set_price_b_cum(
        env,
        get_price_b_cum(env).saturating_add(price_b.saturating_mul(elapsed)),
    );
    set_last_ts(env, now);
}
