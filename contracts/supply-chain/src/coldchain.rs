// Copyright (c) 2026 StellarDevTools
// SPDX-License-Identifier: MIT

//! Cold-chain temperature logging & SLA penalty enforcement (#1272).
//!
//! Adds to the supply chain contract:
//! - Per-product temperature log entries recorded at each checkpoint
//! - Configurable SLA range (min/max temp) per product
//! - Automatic SLA violation detection and penalty slashing of a
//!   handler's deposit when readings fall outside the allowed range

use soroban_sdk::{contracttype, Address};

// ── Temperature log ───────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug)]
pub struct TempLog {
    pub product_id: u32,
    pub index: u32,
    /// Temperature in hundredths of a degree (e.g. 425 = 4.25 C)
    pub temp_hundredths: i64,
    pub logged_by: Address,
    pub timestamp: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SlaRange {
    pub min_hundredths: i64,
    pub max_hundredths: i64,
    /// Penalty (in stroops) deducted from the handler deposit per violation
    pub penalty_per_violation: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct Violation {
    pub product_id: u32,
    pub temp_log_index: u32,
    pub reading: i64,
    pub sla_min: i64,
    pub sla_max: i64,
    pub handler: Address,
    pub penalty_applied: bool,
    pub timestamp: u64,
}

// ── Storage keys ──────────────────────────────────────────────────────────────

#[contracttype]
pub enum TempKey {
    /// SLA configured for a product.
    Sla(u32),
    /// Number of temperature logs for a product.
    TempCount(u32),
    /// A temperature reading.
    TempLog(u32, u32),
    /// Number of violations for a product.
    ViolationCount(u32),
    /// A recorded violation.
    Violation(u32, u32),
    /// Deposit held by a handler for cold-chain compliance.
    HandlerDeposit(Address),
    /// Total penalties accumulated per product.
    PenaltyPool(u32),
}
