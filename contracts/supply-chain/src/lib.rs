// Copyright (c) 2026 StellarDevTools
// SPDX-License-Identifier: MIT

//! # Supply Chain Tracking Contract
//!
//! Tracks products from registration through delivery with:
//! - Provenance verification via metadata hashes
//! - Checkpoint-based traceability (location + handler at each step)
//! - Quality assurance reports by authorised inspectors
//! - Recall mechanism for compromised products

#![no_std]

mod coldchain;
mod storage;
mod test;
mod types;

use soroban_sdk::{contract, contractimpl, Address, Env, String};

use crate::storage::{
    get_admin, get_checkpoint, get_checkpoint_count, get_product, get_product_count,
    get_quality_report, is_handler, is_initialized, is_inspector, set_admin, set_checkpoint,
    set_checkpoint_count, set_handler, set_inspector, set_product, set_product_count,
    set_quality_report,
};
use crate::types::{Checkpoint, Error, Product, ProductStatus, QualityReport, QualityResult};
use crate::coldchain::{SlaRange, TempKey, TempLog, Violation};

#[contract]
pub struct SupplyChain;

#[contractimpl]
impl SupplyChain {
    // ── Initialisation ────────────────────────────────────────────────────────

    pub fn initialize(env: Env, admin: Address) -> Result<(), Error> {
        if is_initialized(&env) {
            return Err(Error::AlreadyInitialized);
        }
        admin.require_auth();
        set_admin(&env, &admin);
        Ok(())
    }

    // ── Role management ───────────────────────────────────────────────────────

    pub fn add_inspector(env: Env, caller: Address, inspector: Address) -> Result<(), Error> {
        caller.require_auth();
        let admin = get_admin(&env)?;
        if caller != admin {
            return Err(Error::Unauthorized);
        }
        set_inspector(&env, &inspector, true);
        Ok(())
    }

    pub fn remove_inspector(env: Env, caller: Address, inspector: Address) -> Result<(), Error> {
        caller.require_auth();
        if caller != get_admin(&env)? {
            return Err(Error::Unauthorized);
        }
        set_inspector(&env, &inspector, false);
        Ok(())
    }

    pub fn add_handler(env: Env, caller: Address, handler: Address) -> Result<(), Error> {
        caller.require_auth();
        if caller != get_admin(&env)? {
            return Err(Error::Unauthorized);
        }
        set_handler(&env, &handler, true);
        Ok(())
    }

    pub fn remove_handler(env: Env, caller: Address, handler: Address) -> Result<(), Error> {
        caller.require_auth();
        if caller != get_admin(&env)? {
            return Err(Error::Unauthorized);
        }
        set_handler(&env, &handler, false);
        Ok(())
    }

    // ── Product registration ──────────────────────────────────────────────────

    /// Register a new product. Returns the new product ID.
    pub fn register_product(
        env: Env,
        owner: Address,
        name: String,
        metadata_hash: u64,
    ) -> Result<u32, Error> {
        owner.require_auth();
        if name.is_empty() {
            return Err(Error::EmptyName);
        }
        let id = get_product_count(&env) + 1;
        let now = env.ledger().timestamp();
        let product = Product {
            id,
            owner,
            name,
            metadata_hash,
            status: ProductStatus::Registered,
            created_at: now,
            updated_at: now,
        };
        set_product(&env, &product);
        set_product_count(&env, id);
        Ok(id)
    }

    // ── Checkpoint / traceability ─────────────────────────────────────────────

    /// Record a supply chain checkpoint (location + handler).
    pub fn add_checkpoint(
        env: Env,
        handler: Address,
        product_id: u32,
        location_hash: u64,
        notes_hash: u64,
    ) -> Result<u32, Error> {
        handler.require_auth();
        if !is_handler(&env, &handler) {
            return Err(Error::NotHandler);
        }
        let mut product = get_product(&env, product_id)?;
        if product.status == ProductStatus::Recalled {
            return Err(Error::AlreadyRecalled);
        }

        let index = get_checkpoint_count(&env, product_id) + 1;
        let now = env.ledger().timestamp();
        let checkpoint = Checkpoint {
            product_id,
            index,
            handler,
            location_hash,
            notes_hash,
            timestamp: now,
        };
        set_checkpoint(&env, &checkpoint);
        set_checkpoint_count(&env, product_id, index);

        product.status = ProductStatus::InTransit;
        product.updated_at = now;
        set_product(&env, &product);
        Ok(index)
    }

    /// Update product status (e.g. AtWarehouse, QualityCheck, Delivered).
    pub fn update_status(
        env: Env,
        caller: Address,
        product_id: u32,
        new_status: ProductStatus,
    ) -> Result<(), Error> {
        caller.require_auth();
        let admin = get_admin(&env)?;
        let is_auth = caller == admin || is_handler(&env, &caller);
        if !is_auth {
            return Err(Error::Unauthorized);
        }
        let mut product = get_product(&env, product_id)?;
        if product.status == ProductStatus::Recalled {
            return Err(Error::AlreadyRecalled);
        }
        product.status = new_status;
        product.updated_at = env.ledger().timestamp();
        set_product(&env, &product);
        Ok(())
    }

    // ── Quality assurance ─────────────────────────────────────────────────────

    /// Submit a quality inspection report.
    pub fn submit_quality_report(
        env: Env,
        inspector: Address,
        product_id: u32,
        result: QualityResult,
        report_hash: u64,
    ) -> Result<(), Error> {
        inspector.require_auth();
        if !is_inspector(&env, &inspector) {
            return Err(Error::NotInspector);
        }
        let mut product = get_product(&env, product_id)?;
        if product.status == ProductStatus::Recalled {
            return Err(Error::AlreadyRecalled);
        }
        let now = env.ledger().timestamp();
        let report = QualityReport {
            product_id,
            inspector,
            result,
            report_hash,
            timestamp: now,
        };
        set_quality_report(&env, &report);

        product.status = match result {
            QualityResult::Pass => ProductStatus::Approved,
            QualityResult::Fail => ProductStatus::Rejected,
            QualityResult::Pending => ProductStatus::QualityCheck,
        };
        product.updated_at = now;
        set_product(&env, &product);
        Ok(())
    }

    // ── Recall ────────────────────────────────────────────────────────────────

    pub fn recall_product(env: Env, caller: Address, product_id: u32) -> Result<(), Error> {
        caller.require_auth();
        if caller != get_admin(&env)? {
            return Err(Error::Unauthorized);
        }
        let mut product = get_product(&env, product_id)?;
        if product.status == ProductStatus::Recalled {
            return Err(Error::AlreadyRecalled);
        }
        product.status = ProductStatus::Recalled;
        product.updated_at = env.ledger().timestamp();
        set_product(&env, &product);
        Ok(())
    }

    // ── Views ─────────────────────────────────────────────────────────────────

    pub fn get_product(env: Env, product_id: u32) -> Result<Product, Error> {
        get_product(&env, product_id)
    }

    pub fn get_checkpoint(
        env: Env,
        product_id: u32,
        index: u32,
    ) -> Result<Checkpoint, Error> {
        get_checkpoint(&env, product_id, index).ok_or(Error::ProductNotFound)
    }

    pub fn get_checkpoint_count(env: Env, product_id: u32) -> u32 {
        get_checkpoint_count(&env, product_id)
    }

    pub fn get_quality_report(env: Env, product_id: u32) -> Result<QualityReport, Error> {
        get_quality_report(&env, product_id)
    }

    pub fn product_count(env: Env) -> u32 {
        get_product_count(&env)
    }

    // ── Cold-chain: temperature logging & SLA penalties (#1272) ──────────────

    /// Configure the acceptable temperature range and penalty for a product.
    /// Only the admin may configure SLAs.
    pub fn set_sla_range(
        env: Env,
        caller: Address,
        product_id: u32,
        min_hundredths: i64,
        max_hundredths: i64,
        penalty_per_violation: i128,
    ) -> Result<(), Error> {
        caller.require_auth();
        if caller != get_admin(&env)? {
            return Err(Error::Unauthorized);
        }
        if min_hundredths >= max_hundredths {
            return Err(Error::InvalidSlaRange);
        }
        get_product(&env, product_id)?;
        let sla = SlaRange {
            min_hundredths,
            max_hundredths,
            penalty_per_violation,
        };
        env.storage().persistent().set(&TempKey::Sla(product_id), &sla);
        Ok(())
    }

    /// Log a temperature reading for a product. Automatically flags an SLA
    /// violation (and records it) when the reading is outside the range.
    /// Returns the index of this log entry.
    pub fn log_temperature(
        env: Env,
        handler: Address,
        product_id: u32,
        temp_hundredths: i64,
    ) -> Result<u32, Error> {
        handler.require_auth();
        if !is_handler(&env, &handler) {
            return Err(Error::NotHandler);
        }
        get_product(&env, product_id)?;

        let index = match env
            .storage()
            .persistent()
            .get::<_, u32>(&TempKey::TempCount(product_id))
        {
            Some(v) => v + 1,
            None => 1,
        };

        let log = TempLog {
            product_id,
            index,
            temp_hundredths,
            logged_by: handler.clone(),
            timestamp: env.ledger().timestamp(),
        };
        env.storage()
            .persistent()
            .set(&TempKey::TempLog(product_id, index), &log);
        env.storage()
            .persistent()
            .set(&TempKey::TempCount(product_id), &index);

        // Automatic SLA check
        if let Some(sla) = env
            .storage()
            .persistent()
            .get::<_, SlaRange>(&TempKey::Sla(product_id))
        {
            if temp_hundredths < sla.min_hundredths
                || temp_hundredths > sla.max_hundredths
            {
                let v_index = match env
                    .storage()
                    .persistent()
                    .get::<_, u32>(&TempKey::ViolationCount(product_id))
                {
                    Some(v) => v + 1,
                    None => 1,
                };
                let violation = Violation {
                    product_id,
                    temp_log_index: index,
                    reading: temp_hundredths,
                    sla_min: sla.min_hundredths,
                    sla_max: sla.max_hundredths,
                    handler: handler.clone(),
                    penalty_applied: false,
                    timestamp: env.ledger().timestamp(),
                };
                env.storage().persistent().set(
                    &TempKey::Violation(product_id, v_index),
                    &violation,
                );
                env.storage().persistent().set(
                    &TempKey::ViolationCount(product_id),
                    &v_index,
                );
            }
        }
        Ok(index)
    }

    /// Apply the SLA penalty for a specific violation. Deducts from the
    /// handler's held deposit and adds to the product's penalty pool.
    /// Anyone may trigger enforcement; only violations not yet penalized
    /// can be processed.
    pub fn enforce_penalty(
        env: Env,
        product_id: u32,
        violation_index: u32,
    ) -> Result<i128, Error> {
        let mut violation: Violation = env
            .storage()
            .persistent()
            .get(&TempKey::Violation(product_id, violation_index))
            .ok_or(Error::ViolationNotFound)?;
        if violation.penalty_applied {
            return Err(Error::PenaltyAlreadyApplied);
        }

        let sla: SlaRange = env
            .storage()
            .persistent()
            .get(&TempKey::Sla(product_id))
            .ok_or(Error::SlaNotConfigured)?;

        let deposit_key = TempKey::HandlerDeposit(violation.handler.clone());
        let deposit: i128 = env
            .storage()
            .persistent()
            .get(&deposit_key)
            .unwrap_or(0);
        let deduction = if deposit < sla.penalty_per_violation {
            deposit
        } else {
            sla.penalty_per_violation
        };
        env.storage().persistent().set(&deposit_key, &(deposit - deduction));

        violation.penalty_applied = true;
        env.storage().persistent().set(
            &TempKey::Violation(product_id, violation_index),
            &violation,
        );

        let pool: i128 = env
            .storage()
            .persistent()
            .get(&TempKey::PenaltyPool(product_id))
            .unwrap_or(0);
        env.storage()
            .persistent()
            .set(&TempKey::PenaltyPool(product_id), &(pool + deduction));
        Ok(deduction)
    }

    /// Handler posts a compliance deposit before transporting cold-chain goods.
    pub fn post_handler_deposit(
        env: Env,
        handler: Address,
        amount: i128,
    ) -> Result<(), Error> {
        handler.require_auth();
        if amount <= 0 {
            return Err(Error::InvalidDeposit);
        }
        let key = TempKey::HandlerDeposit(handler);
        let current: i128 = env.storage().persistent().get(&key).unwrap_or(0);
        env.storage().persistent().set(&key, &(current + amount));
        Ok(())
    }

    /// Read a stored temperature log.
    pub fn get_temperature_log(
        env: Env,
        product_id: u32,
        index: u32,
    ) -> Option<TempLog> {
        env.storage()
            .persistent()
            .get(&TempKey::TempLog(product_id, index))
    }

    /// Number of temperature logs recorded for a product.
    pub fn temperature_log_count(env: Env, product_id: u32) -> u32 {
        env.storage()
            .persistent()
            .get(&TempKey::TempCount(product_id))
            .unwrap_or(0)
    }

    /// Read a recorded violation.
    pub fn get_violation(
        env: Env,
        product_id: u32,
        index: u32,
    ) -> Option<Violation> {
        env.storage()
            .persistent()
            .get(&TempKey::Violation(product_id, index))
    }

    /// Number of violations recorded for a product.
    pub fn violation_count(env: Env, product_id: u32) -> u32 {
        env.storage()
            .persistent()
            .get(&TempKey::ViolationCount(product_id))
            .unwrap_or(0)
    }

    /// Remaining deposit balance of a handler.
    pub fn handler_deposit(env: Env, handler: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&TempKey::HandlerDeposit(handler))
            .unwrap_or(0)
    }
}
