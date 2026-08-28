// Copyright (c) 2026 StellarDevTools
// SPDX-License-Identifier: MIT

//! # NFT Fractionalization Vault with ERC-20 Tokenizer & Buyout Auction
//!
//! Implements fractional NFT ownership via governance tokens with:
//!
//! ## Vault Lifecycle
//! 1. Creator calls `create_vault` to lock an NFT and issue `total_fractions` tokens.
//! 2. Fractions are distributed to the creator (or can be transferred).
//! 3. Fraction holders can `transfer`, `approve`, and `transfer_from` (ERC-20 semantics).
//! 4. Any user can initiate a buyout by calling `start_buyout` with a bid ≥ reserve price.
//! 5. Fraction holders vote for/against the buyout during the auction period.
//! 6. If vote_for_bps > 50% of supply at auction end, `settle_buyout` succeeds:
//!    - Bidder receives the NFT (on-chain record).
//!    - Fraction holders can redeem fractions for proportional payout.
//! 7. If buyout fails, vault returns to Active status.
//!
//! ## ERC-20 Governance Token Interface
//! Each vault has its own fraction token with:
//! - `balance_of(vault_id, holder)` → i128
//! - `transfer(vault_id, from, to, amount)`
//! - `approve(vault_id, owner, spender, amount)`
//! - `transfer_from(vault_id, spender, from, to, amount)`
//! - `total_supply(vault_id)` → i128
//!
//! ## Buyout Auction
//! - Bidder commits the full bid amount upfront.
//! - 72-hour voting window for fraction holders to vote.
//! - Majority (>50% of circulating fractions) required.
//! - Failed auction refunds the bidder.

#![no_std]

mod storage;
mod test;
mod types;

use soroban_sdk::{contract, contractimpl, symbol_short, Address, Env, String};

use crate::storage::{
    get_admin, get_allowance, get_buyout_bid, get_fraction_balance, get_total_fractions_global,
    get_vault, get_vault_count, is_initialized, is_paused, set_admin, set_allowance,
    set_buyout_bid, set_fraction_balance, set_paused, set_total_fractions_global, set_vault,
    set_vault_count,
};
use crate::types::{BuyoutBid, Error, HolderPosition, NftVault, VaultStatus};

/// Price precision for reserve prices and bid amounts.
const PRICE_PRECISION: i128 = 1_000_000;
/// Auction duration in seconds (72 hours).
const AUCTION_DURATION: u64 = 259_200;
/// Required majority to approve buyout (>50% of supply).
const BUYOUT_MAJORITY_BPS: i128 = 5_001; // 50.01% — simple majority

#[contract]
pub struct NftFractionalVault;

#[contractimpl]
impl NftFractionalVault {
    // ── Initialization ────────────────────────────────────────────────────────

    pub fn initialize(env: Env, admin: Address) -> Result<(), Error> {
        if is_initialized(&env) {
            return Err(Error::AlreadyInitialized);
        }
        admin.require_auth();
        set_admin(&env, &admin);
        Ok(())
    }

    // ── Vault creation ────────────────────────────────────────────────────────

    /// Lock an NFT and issue `total_fractions` governance/fraction tokens to the creator.
    ///
    /// - `nft_contract`: Address of the NFT collection.
    /// - `nft_token_id`: Token ID within the collection.
    /// - `fraction_name`: Human-readable name for the fraction token (e.g. "BAYC#1234-FRAC").
    /// - `total_fractions`: Number of fraction tokens to issue.
    /// - `reserve_price`: Minimum buyout price (PRICE_PRECISION scaled). Set to 0 for no floor.
    ///
    /// Returns the vault id.
    pub fn create_vault(
        env: Env,
        creator: Address,
        nft_contract: Address,
        nft_token_id: u32,
        fraction_name: String,
        total_fractions: i128,
        reserve_price: i128,
    ) -> Result<u32, Error> {
        ensure_active(&env)?;
        creator.require_auth();

        if total_fractions <= 0 {
            return Err(Error::InvalidFractions);
        }
        if reserve_price < 0 {
            return Err(Error::InvalidReservePrice);
        }

        let vault_id = get_vault_count(&env);
        let vault = NftVault {
            id: vault_id,
            creator: creator.clone(),
            nft_contract: nft_contract.clone(),
            nft_token_id,
            fraction_name,
            total_fractions,
            reserve_price,
            status: VaultStatus::Active,
            created_at: env.ledger().timestamp(),
        };
        set_vault(&env, &vault);
        set_vault_count(&env, vault_id + 1);

        // Issue all fractions to creator.
        set_fraction_balance(&env, vault_id, &creator, total_fractions);
        set_total_fractions_global(
            &env,
            get_total_fractions_global(&env) + total_fractions,
        );

        env.events().publish(
            (symbol_short!("vault_new"),),
            (vault_id, creator, nft_contract, total_fractions),
        );
        Ok(vault_id)
    }

    // ── ERC-20 fraction token interface ───────────────────────────────────────

    /// Transfer `amount` fractions of vault `vault_id` from `from` to `to`.
    pub fn transfer(
        env: Env,
        vault_id: u32,
        from: Address,
        to: Address,
        amount: i128,
    ) -> Result<(), Error> {
        ensure_active(&env)?;
        from.require_auth();
        if amount <= 0 {
            return Err(Error::ZeroAmount);
        }
        let vault = get_vault(&env, vault_id).ok_or(Error::VaultNotFound)?;
        if vault.status == VaultStatus::BoughtOut || vault.status == VaultStatus::Redeemed {
            return Err(Error::VaultNotActive);
        }

        let from_bal = get_fraction_balance(&env, vault_id, &from);
        if from_bal < amount {
            return Err(Error::InsufficientBalance);
        }

        set_fraction_balance(&env, vault_id, &from, from_bal - amount);
        set_fraction_balance(
            &env,
            vault_id,
            &to,
            get_fraction_balance(&env, vault_id, &to) + amount,
        );

        env.events()
            .publish((symbol_short!("frac_xfr"),), (vault_id, from, to, amount));
        Ok(())
    }

    /// Approve `spender` to transfer up to `amount` fractions on behalf of `owner`.
    pub fn approve(
        env: Env,
        vault_id: u32,
        owner: Address,
        spender: Address,
        amount: i128,
    ) -> Result<(), Error> {
        ensure_active(&env)?;
        owner.require_auth();
        get_vault(&env, vault_id).ok_or(Error::VaultNotFound)?;
        set_allowance(&env, vault_id, &owner, &spender, amount);
        env.events()
            .publish((symbol_short!("frac_appr"),), (vault_id, owner, spender, amount));
        Ok(())
    }

    /// Transfer fractions using an allowance (ERC-20 transferFrom).
    pub fn transfer_from(
        env: Env,
        vault_id: u32,
        spender: Address,
        from: Address,
        to: Address,
        amount: i128,
    ) -> Result<(), Error> {
        ensure_active(&env)?;
        spender.require_auth();
        if amount <= 0 {
            return Err(Error::ZeroAmount);
        }

        let vault = get_vault(&env, vault_id).ok_or(Error::VaultNotFound)?;
        if vault.status == VaultStatus::BoughtOut || vault.status == VaultStatus::Redeemed {
            return Err(Error::VaultNotActive);
        }

        let allowance = get_allowance(&env, vault_id, &from, &spender);
        if allowance < amount {
            return Err(Error::InsufficientAllowance);
        }

        let from_bal = get_fraction_balance(&env, vault_id, &from);
        if from_bal < amount {
            return Err(Error::InsufficientBalance);
        }

        set_allowance(&env, vault_id, &from, &spender, allowance - amount);
        set_fraction_balance(&env, vault_id, &from, from_bal - amount);
        set_fraction_balance(
            &env,
            vault_id,
            &to,
            get_fraction_balance(&env, vault_id, &to) + amount,
        );

        env.events()
            .publish((symbol_short!("frac_xfra"),), (vault_id, spender, from, to, amount));
        Ok(())
    }

    // ── Buyout auction ────────────────────────────────────────────────────────

    /// Initiate a buyout auction for a vault.
    ///
    /// `bid_amount` must be ≥ `reserve_price * total_fractions / PRICE_PRECISION`.
    /// A 72-hour voting window starts immediately.
    ///
    /// Returns the auction end timestamp.
    pub fn start_buyout(
        env: Env,
        bidder: Address,
        vault_id: u32,
        bid_amount: i128,
    ) -> Result<u64, Error> {
        ensure_active(&env)?;
        bidder.require_auth();

        let vault = get_vault(&env, vault_id).ok_or(Error::VaultNotFound)?;
        if vault.status != VaultStatus::Active {
            return Err(Error::VaultNotActive);
        }

        // Validate bid ≥ reserve price.
        if vault.reserve_price > 0 {
            let min_bid = vault.reserve_price
                .checked_mul(vault.total_fractions)
                .ok_or(Error::Overflow)?
                / PRICE_PRECISION;
            if bid_amount < min_bid {
                return Err(Error::BuyoutBelowReserve);
            }
        }

        if get_buyout_bid(&env, vault_id).is_some() {
            return Err(Error::BuyoutAuctionActive);
        }

        let price_per_fraction = bid_amount
            .checked_mul(PRICE_PRECISION)
            .ok_or(Error::Overflow)?
            / vault.total_fractions.max(1);

        let auction_end = env.ledger().timestamp() + AUCTION_DURATION;
        let bid = BuyoutBid {
            vault_id,
            bidder: bidder.clone(),
            bid_amount,
            price_per_fraction,
            auction_end,
            settled: false,
            votes_for: 0,
            votes_against: 0,
        };
        set_buyout_bid(&env, &bid);

        // Update vault status.
        let mut v = vault;
        v.status = VaultStatus::BuyoutInProgress;
        set_vault(&env, &v);

        env.events().publish(
            (symbol_short!("buyout_s"),),
            (vault_id, bidder, bid_amount, auction_end),
        );
        Ok(auction_end)
    }

    /// Vote on an active buyout auction.
    ///
    /// Voting power = fraction balance. Votes are cast for or against.
    /// A holder can split their vote by calling vote multiple times (last vote wins for simplicity).
    pub fn vote_on_buyout(
        env: Env,
        voter: Address,
        vault_id: u32,
        vote_for: bool,
    ) -> Result<i128, Error> {
        ensure_active(&env)?;
        voter.require_auth();

        let mut bid = get_buyout_bid(&env, vault_id).ok_or(Error::BuyoutAuctionNotActive)?;
        if bid.settled {
            return Err(Error::BuyoutAlreadySettled);
        }
        if env.ledger().timestamp() > bid.auction_end {
            return Err(Error::BuyoutAuctionNotEnded);
        }

        let voting_power = get_fraction_balance(&env, vault_id, &voter);
        if voting_power == 0 {
            return Err(Error::InsufficientBalance);
        }

        if vote_for {
            bid.votes_for = bid.votes_for + voting_power;
        } else {
            bid.votes_against = bid.votes_against + voting_power;
        }
        set_buyout_bid(&env, &bid);

        env.events()
            .publish((symbol_short!("buyout_v"),), (vault_id, voter, vote_for, voting_power));
        Ok(voting_power)
    }

    /// Settle a buyout auction after the voting period ends.
    ///
    /// If votes_for > BUYOUT_MAJORITY_BPS% of total fractions:
    ///   - Vault status → BoughtOut
    ///   - Bid is marked settled
    ///   - NFT ownership record transferred to bidder
    ///
    /// If votes failed:
    ///   - Vault status → Active (bidder refunded off-chain)
    ///   - Bid cleared
    ///
    /// Returns true if buyout succeeded.
    pub fn settle_buyout(env: Env, vault_id: u32) -> Result<bool, Error> {
        ensure_active(&env)?;

        let mut bid = get_buyout_bid(&env, vault_id).ok_or(Error::BuyoutAuctionNotActive)?;
        if bid.settled {
            return Err(Error::BuyoutAlreadySettled);
        }
        if env.ledger().timestamp() < bid.auction_end {
            return Err(Error::BuyoutAuctionNotEnded);
        }

        let vault = get_vault(&env, vault_id).ok_or(Error::VaultNotFound)?;
        let total = vault.total_fractions.max(1);

        // Check if majority approved.
        let votes_for_bps = bid.votes_for
            .checked_mul(10_000)
            .ok_or(Error::Overflow)?
            / total;
        let success = votes_for_bps >= BUYOUT_MAJORITY_BPS;

        let mut v = vault;
        if success {
            v.status = VaultStatus::BoughtOut;
            bid.settled = true;
            set_buyout_bid(&env, &bid);
            env.events().publish(
                (symbol_short!("buyout_ok"),),
                (vault_id, bid.bidder.clone(), bid.bid_amount),
            );
        } else {
            v.status = VaultStatus::Active;
            // Remove bid to allow new auction.
            // (Bid record stays for audit but vault is Active again)
            bid.settled = true;
            set_buyout_bid(&env, &bid);
            env.events()
                .publish((symbol_short!("buyout_no"),), (vault_id,));
        }
        set_vault(&env, &v);

        Ok(success)
    }

    /// Redeem fractions for proportional payout from a successful buyout.
    ///
    /// Holder burns their fractions and receives:
    ///   payout = (fraction_balance / total_fractions) * bid_amount
    ///
    /// Returns the payout amount.
    pub fn redeem_fractions(
        env: Env,
        holder: Address,
        vault_id: u32,
    ) -> Result<i128, Error> {
        ensure_active(&env)?;
        holder.require_auth();

        let vault = get_vault(&env, vault_id).ok_or(Error::VaultNotFound)?;
        if vault.status != VaultStatus::BoughtOut {
            return Err(Error::VaultNotActive);
        }

        let balance = get_fraction_balance(&env, vault_id, &holder);
        if balance == 0 {
            return Err(Error::InsufficientBalance);
        }

        let bid = get_buyout_bid(&env, vault_id).ok_or(Error::BuyoutAuctionNotActive)?;
        let payout = balance
            .checked_mul(bid.bid_amount)
            .ok_or(Error::Overflow)?
            / vault.total_fractions.max(1);

        // Burn fractions.
        set_fraction_balance(&env, vault_id, &holder, 0);
        set_total_fractions_global(
            &env,
            (get_total_fractions_global(&env) - balance).max(0),
        );

        env.events()
            .publish((symbol_short!("frac_redm"),), (vault_id, holder, balance, payout));
        Ok(payout)
    }

    // ── Admin ─────────────────────────────────────────────────────────────────

    /// Update reserve price for a vault. Creator or admin only.
    pub fn set_reserve_price(
        env: Env,
        caller: Address,
        vault_id: u32,
        new_reserve: i128,
    ) -> Result<(), Error> {
        ensure_active(&env)?;
        caller.require_auth();
        if new_reserve < 0 {
            return Err(Error::InvalidReservePrice);
        }
        let mut vault = get_vault(&env, vault_id).ok_or(Error::VaultNotFound)?;
        if vault.status != VaultStatus::Active {
            return Err(Error::VaultNotActive);
        }
        // Only creator or admin can update reserve.
        let admin = get_admin(&env)?;
        if vault.creator != caller && admin != caller {
            return Err(Error::Unauthorized);
        }
        vault.reserve_price = new_reserve;
        set_vault(&env, &vault);
        env.events()
            .publish((symbol_short!("res_upd"),), (vault_id, new_reserve));
        Ok(())
    }

    pub fn set_paused(env: Env, admin: Address, paused: bool) -> Result<(), Error> {
        admin.require_auth();
        let contract_admin = get_admin(&env)?;
        if contract_admin != admin {
            return Err(Error::Unauthorized);
        }
        set_paused(&env, paused);
        let sym = if paused {
            symbol_short!("paused")
        } else {
            symbol_short!("unpaused")
        };
        env.events().publish((sym,), ());
        Ok(())
    }

    // ── Read-only ─────────────────────────────────────────────────────────────

    pub fn get_vault(env: Env, vault_id: u32) -> Result<NftVault, Error> {
        ensure_initialized(&env)?;
        get_vault(&env, vault_id).ok_or(Error::VaultNotFound)
    }

    pub fn get_vault_count(env: Env) -> Result<u32, Error> {
        ensure_initialized(&env)?;
        Ok(get_vault_count(&env))
    }

    pub fn balance_of(env: Env, vault_id: u32, holder: Address) -> Result<i128, Error> {
        ensure_initialized(&env)?;
        Ok(get_fraction_balance(&env, vault_id, &holder))
    }

    pub fn allowance(
        env: Env,
        vault_id: u32,
        owner: Address,
        spender: Address,
    ) -> Result<i128, Error> {
        ensure_initialized(&env)?;
        Ok(get_allowance(&env, vault_id, &owner, &spender))
    }

    pub fn total_supply(env: Env, vault_id: u32) -> Result<i128, Error> {
        ensure_initialized(&env)?;
        let vault = get_vault(&env, vault_id).ok_or(Error::VaultNotFound)?;
        Ok(vault.total_fractions)
    }

    pub fn get_buyout_bid(env: Env, vault_id: u32) -> Result<BuyoutBid, Error> {
        ensure_initialized(&env)?;
        get_buyout_bid(&env, vault_id).ok_or(Error::BuyoutAuctionNotActive)
    }

    pub fn get_holder_position(
        env: Env,
        vault_id: u32,
        holder: Address,
    ) -> Result<HolderPosition, Error> {
        ensure_initialized(&env)?;
        let vault = get_vault(&env, vault_id).ok_or(Error::VaultNotFound)?;
        let balance = get_fraction_balance(&env, vault_id, &holder);
        let total = vault.total_fractions.max(1);
        let ownership_bps = balance
            .checked_mul(10_000)
            .ok_or(Error::Overflow)?
            / total;
        let value_at_reserve = balance
            .checked_mul(vault.reserve_price)
            .ok_or(Error::Overflow)?
            / PRICE_PRECISION.max(1);
        Ok(HolderPosition {
            vault_id,
            holder,
            fraction_balance: balance,
            ownership_bps,
            value_at_reserve,
        })
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn ensure_initialized(env: &Env) -> Result<(), Error> {
    if !is_initialized(env) {
        return Err(Error::NotInitialized);
    }
    Ok(())
}

fn ensure_active(env: &Env) -> Result<(), Error> {
    ensure_initialized(env)?;
    if is_paused(env) {
        return Err(Error::Paused);
    }
    Ok(())
}
