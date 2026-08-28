#![cfg_attr(not(test), no_std)]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, token, Address, Env, Symbol, Vec, U256,
};

const BPS_DENOMINATOR: i128 = 10_000;
const MAX_PSM_FEE_BPS: u32 = 500;
const TTL_THRESHOLD: u32 = 864_000;
const TTL_BUMP: u32 = 1_728_000;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    Unauthorized = 1,
    ContractPaused = 2,
    AlreadyInState = 3,
    NotInitialized = 4,
    InvalidAmount = 5,
    InsufficientBalance = 6,
    PriceStale = 7,
    RebaseTooFrequent = 8,
    PsmNotConfigured = 9,
    PsmAlreadyConfigured = 10,
    UnsupportedCollateral = 11,
    InvalidFee = 12,
    SlippageExceeded = 13,
    ArithmeticOverflow = 14,
    InsufficientReserve = 15,
    InsufficientPsmSupply = 16,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    TargetPrice,
    CurrentPrice,
    TotalSupply,
    ShareSupply,
    UserShares(Address),
    UserTokens(Address),
    Paused,
    LastRebaseTime,
    ReserveBalance,
    OracleAddress,
    RebaseCooldown,
    PsmConfig,
    PsmSupply,
    CollateralReserve(Address),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PsmConfig {
    pub burn_fee_bps: u32,
    pub mint_fee_bps: u32,
    pub usdc: Address,
    pub usdt: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SwapResult {
    pub amount_in: i128,
    pub amount_out: i128,
    pub collateral: Address,
    pub fee: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RebaseInfo {
    pub old_supply: i128,
    pub new_supply: i128,
    pub price: i128,
    pub timestamp: u64,
}

#[contract]
pub struct AlgorithmicStablecoin;

#[contractimpl]
impl AlgorithmicStablecoin {
    pub fn init(env: Env, admin: Address, oracle: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInState);
        }

        admin.require_auth();

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::OracleAddress, &oracle);
        env.storage()
            .instance()
            .set(&DataKey::TargetPrice, &10_000_000i128);
        env.storage()
            .instance()
            .set(&DataKey::CurrentPrice, &10_000_000i128);
        env.storage().instance().set(&DataKey::TotalSupply, &0i128);
        env.storage()
            .instance()
            .set(&DataKey::ShareSupply, &1_000_000_000i128);
        env.storage().instance().set(&DataKey::Paused, &false);
        env.storage()
            .instance()
            .set(&DataKey::LastRebaseTime, &0u64);
        env.storage()
            .instance()
            .set(&DataKey::ReserveBalance, &0i128);
        env.storage()
            .instance()
            .set(&DataKey::RebaseCooldown, &3600u64);

        env.events()
            .publish((Symbol::new(&env, "initialized"),), (admin, oracle));

        Ok(())
    }

    pub fn mint(env: Env, admin: Address, to: Address, amount: i128) -> Result<(), Error> {
        admin.require_auth();
        Self::assert_admin(&env, &admin)?;
        Self::assert_not_paused(&env)?;

        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        let current_balance: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::UserTokens(to.clone()))
            .unwrap_or(0);
        env.storage().persistent().set(
            &DataKey::UserTokens(to.clone()),
            &(current_balance + amount),
        );

        let total_supply: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalSupply)
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&DataKey::TotalSupply, &(total_supply + amount));

        env.events()
            .publish((Symbol::new(&env, "mint"),), (to, amount));

        Ok(())
    }

    pub fn burn(env: Env, from: Address, amount: i128) -> Result<(), Error> {
        from.require_auth();
        Self::assert_not_paused(&env)?;

        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        let current_balance: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::UserTokens(from.clone()))
            .unwrap_or(0);
        if current_balance < amount {
            return Err(Error::InsufficientBalance);
        }

        env.storage().persistent().set(
            &DataKey::UserTokens(from.clone()),
            &(current_balance - amount),
        );

        let total_supply: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalSupply)
            .unwrap_or(0);
        let new_supply = total_supply
            .checked_sub(amount)
            .ok_or(Error::ArithmeticOverflow)?;
        env.storage()
            .instance()
            .set(&DataKey::TotalSupply, &new_supply);

        // Stablecoins are fungible, so a legacy burn also reduces the PSM's
        // outstanding collateral-backed liability.
        if env.storage().instance().has(&DataKey::PsmConfig) {
            let psm_supply: i128 = env
                .storage()
                .instance()
                .get(&DataKey::PsmSupply)
                .unwrap_or(0);
            env.storage()
                .instance()
                .set(&DataKey::PsmSupply, &psm_supply.saturating_sub(amount));
            bump_instance_ttl(&env);
        }

        env.events()
            .publish((Symbol::new(&env, "burn"),), (from, amount));

        Ok(())
    }

    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) -> Result<(), Error> {
        from.require_auth();
        Self::assert_not_paused(&env)?;

        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        let from_balance: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::UserTokens(from.clone()))
            .unwrap_or(0);
        if from_balance < amount {
            return Err(Error::InsufficientBalance);
        }

        let to_balance: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::UserTokens(to.clone()))
            .unwrap_or(0);

        env.storage()
            .persistent()
            .set(&DataKey::UserTokens(from.clone()), &(from_balance - amount));
        env.storage()
            .persistent()
            .set(&DataKey::UserTokens(to.clone()), &(to_balance + amount));

        env.events()
            .publish((Symbol::new(&env, "transfer"),), (from, to, amount));

        Ok(())
    }

    pub fn set_price(env: Env, oracle: Address, new_price: i128) -> Result<(), Error> {
        oracle.require_auth();
        Self::assert_oracle(&env, &oracle)?;

        if new_price <= 0 {
            return Err(Error::InvalidAmount);
        }

        env.storage()
            .instance()
            .set(&DataKey::CurrentPrice, &new_price);

        env.events().publish(
            (Symbol::new(&env, "price_updated"),),
            (oracle, new_price, env.ledger().timestamp()),
        );

        Ok(())
    }

    pub fn rebase(env: Env, caller: Address) -> Result<RebaseInfo, Error> {
        caller.require_auth();
        Self::assert_not_paused(&env)?;

        let last_rebase: u64 = env
            .storage()
            .instance()
            .get(&DataKey::LastRebaseTime)
            .unwrap_or(0);
        let cooldown: u64 = env
            .storage()
            .instance()
            .get(&DataKey::RebaseCooldown)
            .unwrap_or(3600);
        let current_time = env.ledger().timestamp();

        if current_time < last_rebase + cooldown {
            return Err(Error::RebaseTooFrequent);
        }

        let current_price: i128 = env
            .storage()
            .instance()
            .get(&DataKey::CurrentPrice)
            .unwrap();
        let target_price: i128 = env.storage().instance().get(&DataKey::TargetPrice).unwrap();
        let old_supply: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalSupply)
            .unwrap_or(0);

        let new_supply = if current_price > target_price {
            let expansion_ratio = (current_price - target_price) * 1_000_000 / target_price;
            let expansion_amount = old_supply * expansion_ratio / 1_000_000;
            old_supply + expansion_amount
        } else if current_price < target_price {
            let contraction_ratio = (target_price - current_price) * 1_000_000 / target_price;
            let max_contraction = old_supply * contraction_ratio / 1_000_000;
            let reserve: i128 = env
                .storage()
                .instance()
                .get(&DataKey::ReserveBalance)
                .unwrap_or(0);
            let actual_contraction = if max_contraction > reserve {
                reserve
            } else {
                max_contraction
            };
            let psm_supply: i128 = env
                .storage()
                .instance()
                .get(&DataKey::PsmSupply)
                .unwrap_or(0);
            (old_supply - actual_contraction).max(psm_supply)
        } else {
            old_supply
        };

        env.storage()
            .instance()
            .set(&DataKey::TotalSupply, &new_supply);
        env.storage()
            .instance()
            .set(&DataKey::LastRebaseTime, &current_time);

        let info = RebaseInfo {
            old_supply,
            new_supply,
            price: current_price,
            timestamp: current_time,
        };

        env.events()
            .publish((Symbol::new(&env, "rebase"),), info.clone());

        Ok(info)
    }

    pub fn pause(env: Env, admin: Address) -> Result<(), Error> {
        admin.require_auth();
        Self::assert_admin(&env, &admin)?;

        if Self::is_paused(&env) {
            return Err(Error::AlreadyInState);
        }

        env.storage().instance().set(&DataKey::Paused, &true);

        env.events().publish((Symbol::new(&env, "paused"),), admin);

        Ok(())
    }

    pub fn unpause(env: Env, admin: Address) -> Result<(), Error> {
        admin.require_auth();
        Self::assert_admin(&env, &admin)?;

        if !Self::is_paused(&env) {
            return Err(Error::AlreadyInState);
        }

        env.storage().instance().set(&DataKey::Paused, &false);

        env.events()
            .publish((Symbol::new(&env, "unpaused"),), admin);

        Ok(())
    }

    pub fn add_reserve(env: Env, admin: Address, amount: i128) -> Result<(), Error> {
        admin.require_auth();
        Self::assert_admin(&env, &admin)?;

        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        let current_reserve: i128 = env
            .storage()
            .instance()
            .get(&DataKey::ReserveBalance)
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&DataKey::ReserveBalance, &(current_reserve + amount));

        env.events()
            .publish((Symbol::new(&env, "reserve_added"),), (admin, amount));

        Ok(())
    }

    pub fn withdraw_reserve(env: Env, admin: Address, amount: i128) -> Result<(), Error> {
        admin.require_auth();
        Self::assert_admin(&env, &admin)?;

        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        let current_reserve: i128 = env
            .storage()
            .instance()
            .get(&DataKey::ReserveBalance)
            .unwrap_or(0);
        if current_reserve < amount {
            return Err(Error::InsufficientBalance);
        }

        env.storage()
            .instance()
            .set(&DataKey::ReserveBalance, &(current_reserve - amount));

        env.events()
            .publish((Symbol::new(&env, "reserve_withdrawn"),), (admin, amount));

        Ok(())
    }

    /// Enables the reserve-backed PSM without changing the legacy initializer.
    /// This can only be configured once so collateral cannot be stranded by a
    /// later address change.
    pub fn configure_psm(
        env: Env,
        admin: Address,
        usdc: Address,
        usdt: Address,
        mint_fee_bps: u32,
        burn_fee_bps: u32,
    ) -> Result<(), Error> {
        admin.require_auth();
        Self::assert_admin(&env, &admin)?;
        if env.storage().instance().has(&DataKey::PsmConfig) {
            return Err(Error::PsmAlreadyConfigured);
        }
        if usdc == usdt {
            return Err(Error::UnsupportedCollateral);
        }
        validate_fees(mint_fee_bps, burn_fee_bps)?;
        let config = PsmConfig {
            burn_fee_bps,
            mint_fee_bps,
            usdc,
            usdt,
        };
        env.storage().instance().set(&DataKey::PsmConfig, &config);
        env.storage().instance().set(&DataKey::PsmSupply, &0i128);
        bump_instance_ttl(&env);
        env.events()
            .publish((Symbol::new(&env, "psm_configured"),), config);
        Ok(())
    }

    /// Updates dynamic swap fees. Fees are capped at 5% to protect users from
    /// accidental or malicious configuration.
    pub fn set_psm_fees(
        env: Env,
        admin: Address,
        mint_fee_bps: u32,
        burn_fee_bps: u32,
    ) -> Result<(), Error> {
        admin.require_auth();
        Self::assert_admin(&env, &admin)?;
        validate_fees(mint_fee_bps, burn_fee_bps)?;
        let mut config = psm_config(&env)?;
        config.mint_fee_bps = mint_fee_bps;
        config.burn_fee_bps = burn_fee_bps;
        env.storage().instance().set(&DataKey::PsmConfig, &config);
        bump_instance_ttl(&env);
        env.events().publish(
            (Symbol::new(&env, "psm_fees_updated"),),
            (mint_fee_bps, burn_fee_bps),
        );
        Ok(())
    }

    /// Swaps supported collateral into stablecoins at 1:1 less the mint fee.
    pub fn psm_mint(
        env: Env,
        user: Address,
        collateral: Address,
        amount: i128,
        min_stable_out: i128,
    ) -> Result<SwapResult, Error> {
        user.require_auth();
        Self::assert_not_paused(&env)?;
        if amount <= 0 || min_stable_out < 0 {
            return Err(Error::InvalidAmount);
        }
        let config = psm_config(&env)?;
        assert_collateral(&config, &collateral)?;
        let fee = fee_ceil(&env, amount, config.mint_fee_bps)?;
        let amount_out = amount.checked_sub(fee).ok_or(Error::ArithmeticOverflow)?;
        if amount_out <= 0 || amount_out < min_stable_out {
            return Err(Error::SlippageExceeded);
        }

        let reserve_key = DataKey::CollateralReserve(collateral.clone());
        let reserve: i128 = env.storage().instance().get(&reserve_key).unwrap_or(0);
        let psm_supply: i128 = env
            .storage()
            .instance()
            .get(&DataKey::PsmSupply)
            .unwrap_or(0);
        let total_supply: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalSupply)
            .unwrap_or(0);
        let balance_key = DataKey::UserTokens(user.clone());
        let balance: i128 = env.storage().persistent().get(&balance_key).unwrap_or(0);

        token::Client::new(&env, &collateral).transfer(
            &user,
            &env.current_contract_address(),
            &amount,
        );
        env.storage().instance().set(
            &reserve_key,
            &reserve
                .checked_add(amount)
                .ok_or(Error::ArithmeticOverflow)?,
        );
        env.storage().instance().set(
            &DataKey::PsmSupply,
            &psm_supply
                .checked_add(amount_out)
                .ok_or(Error::ArithmeticOverflow)?,
        );
        env.storage().instance().set(
            &DataKey::TotalSupply,
            &total_supply
                .checked_add(amount_out)
                .ok_or(Error::ArithmeticOverflow)?,
        );
        set_user_balance(
            &env,
            &balance_key,
            balance
                .checked_add(amount_out)
                .ok_or(Error::ArithmeticOverflow)?,
        );
        bump_instance_ttl(&env);
        let result = SwapResult {
            amount_in: amount,
            amount_out,
            collateral,
            fee,
        };
        env.events()
            .publish((Symbol::new(&env, "psm_mint"), user), result.clone());
        Ok(result)
    }

    /// Burns stablecoins for supported collateral at 1:1 less the burn fee.
    pub fn psm_burn(
        env: Env,
        user: Address,
        collateral: Address,
        stable_amount: i128,
        min_collateral_out: i128,
    ) -> Result<SwapResult, Error> {
        user.require_auth();
        Self::assert_not_paused(&env)?;
        if stable_amount <= 0 || min_collateral_out < 0 {
            return Err(Error::InvalidAmount);
        }
        let config = psm_config(&env)?;
        assert_collateral(&config, &collateral)?;
        let fee = fee_ceil(&env, stable_amount, config.burn_fee_bps)?;
        let amount_out = stable_amount
            .checked_sub(fee)
            .ok_or(Error::ArithmeticOverflow)?;
        if amount_out <= 0 || amount_out < min_collateral_out {
            return Err(Error::SlippageExceeded);
        }

        let balance_key = DataKey::UserTokens(user.clone());
        let balance: i128 = env.storage().persistent().get(&balance_key).unwrap_or(0);
        if balance < stable_amount {
            return Err(Error::InsufficientBalance);
        }
        let psm_supply: i128 = env
            .storage()
            .instance()
            .get(&DataKey::PsmSupply)
            .unwrap_or(0);
        if psm_supply < stable_amount {
            return Err(Error::InsufficientPsmSupply);
        }
        let reserve_key = DataKey::CollateralReserve(collateral.clone());
        let reserve: i128 = env.storage().instance().get(&reserve_key).unwrap_or(0);
        if reserve < amount_out {
            return Err(Error::InsufficientReserve);
        }
        let total_supply: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalSupply)
            .unwrap_or(0);

        set_user_balance(&env, &balance_key, balance - stable_amount);
        env.storage()
            .instance()
            .set(&DataKey::PsmSupply, &(psm_supply - stable_amount));
        env.storage()
            .instance()
            .set(&DataKey::TotalSupply, &(total_supply - stable_amount));
        env.storage()
            .instance()
            .set(&reserve_key, &(reserve - amount_out));
        bump_instance_ttl(&env);
        token::Client::new(&env, &collateral).transfer(
            &env.current_contract_address(),
            &user,
            &amount_out,
        );

        let result = SwapResult {
            amount_in: stable_amount,
            amount_out,
            collateral,
            fee,
        };
        env.events()
            .publish((Symbol::new(&env, "psm_burn"), user), result.clone());
        Ok(result)
    }

    pub fn get_psm_config(env: Env) -> Result<PsmConfig, Error> {
        let config = psm_config(&env)?;
        bump_instance_ttl(&env);
        Ok(config)
    }

    pub fn get_collateral_reserve(env: Env, collateral: Address) -> Result<i128, Error> {
        let config = psm_config(&env)?;
        assert_collateral(&config, &collateral)?;
        bump_instance_ttl(&env);
        Ok(env
            .storage()
            .instance()
            .get(&DataKey::CollateralReserve(collateral))
            .unwrap_or(0))
    }

    pub fn get_psm_supply(env: Env) -> Result<i128, Error> {
        psm_config(&env)?;
        bump_instance_ttl(&env);
        Ok(env
            .storage()
            .instance()
            .get(&DataKey::PsmSupply)
            .unwrap_or(0))
    }

    pub fn balance(env: Env, user: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::UserTokens(user))
            .unwrap_or(0)
    }

    pub fn total_supply(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::TotalSupply)
            .unwrap_or(0)
    }

    pub fn get_price(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::CurrentPrice)
            .unwrap_or(10_000_000)
    }

    pub fn get_target_price(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::TargetPrice)
            .unwrap_or(10_000_000)
    }

    pub fn get_reserve(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::ReserveBalance)
            .unwrap_or(0)
    }

    pub fn is_paused(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::Paused)
            .unwrap_or(false)
    }

    pub fn get_admin(env: Env) -> Result<Address, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)
    }

    fn assert_admin(env: &Env, caller: &Address) -> Result<(), Error> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;

        if &admin != caller {
            return Err(Error::Unauthorized);
        }
        Ok(())
    }

    fn assert_oracle(env: &Env, caller: &Address) -> Result<(), Error> {
        let oracle: Address = env
            .storage()
            .instance()
            .get(&DataKey::OracleAddress)
            .ok_or(Error::NotInitialized)?;

        if &oracle != caller {
            return Err(Error::Unauthorized);
        }
        Ok(())
    }

    fn assert_not_paused(env: &Env) -> Result<(), Error> {
        if Self::is_paused(env.clone()) {
            return Err(Error::ContractPaused);
        }
        Ok(())
    }
}

fn validate_fees(mint_fee_bps: u32, burn_fee_bps: u32) -> Result<(), Error> {
    if mint_fee_bps > MAX_PSM_FEE_BPS || burn_fee_bps > MAX_PSM_FEE_BPS {
        return Err(Error::InvalidFee);
    }
    Ok(())
}

fn psm_config(env: &Env) -> Result<PsmConfig, Error> {
    env.storage()
        .instance()
        .get(&DataKey::PsmConfig)
        .ok_or(Error::PsmNotConfigured)
}

fn assert_collateral(config: &PsmConfig, collateral: &Address) -> Result<(), Error> {
    if collateral != &config.usdc && collateral != &config.usdt {
        return Err(Error::UnsupportedCollateral);
    }
    Ok(())
}

fn fee_ceil(env: &Env, amount: i128, fee_bps: u32) -> Result<i128, Error> {
    if fee_bps == 0 {
        return Ok(0);
    }
    let numerator = U256::from_u128(env, amount as u128)
        .mul(&U256::from_u128(env, fee_bps as u128))
        .add(&U256::from_u128(env, (BPS_DENOMINATOR - 1) as u128));
    let fee = numerator
        .div(&U256::from_u128(env, BPS_DENOMINATOR as u128))
        .to_u128()
        .ok_or(Error::ArithmeticOverflow)?;
    i128::try_from(fee).map_err(|_| Error::ArithmeticOverflow)
}

fn set_user_balance(env: &Env, key: &DataKey, balance: i128) {
    env.storage().persistent().set(key, &balance);
    env.storage()
        .persistent()
        .extend_ttl(key, TTL_THRESHOLD, TTL_BUMP);
}

fn bump_instance_ttl(env: &Env) {
    env.storage().instance().extend_ttl(TTL_THRESHOLD, TTL_BUMP);
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{
        testutils::Address as _,
        token::{Client as TokenClient, StellarAssetClient},
        Env,
    };

    fn setup() -> (
        Env,
        Address,
        Address,
        Address,
        AlgorithmicStablecoinClient<'static>,
    ) {
        let env = Env::default();
        env.mock_all_auths();
        let id = env.register_contract(None, AlgorithmicStablecoin);
        let client = AlgorithmicStablecoinClient::new(&env, &id);
        let admin = Address::generate(&env);
        let oracle = Address::generate(&env);
        let user = Address::generate(&env);

        client.init(&admin, &oracle);

        let env = std::boxed::Box::leak(std::boxed::Box::new(env));
        let client = AlgorithmicStablecoinClient::new(env, &id);

        (env.clone(), admin, oracle, user, client)
    }

    #[test]
    fn test_init() {
        let env = Env::default();
        env.mock_all_auths();
        let id = env.register_contract(None, AlgorithmicStablecoin);
        let client = AlgorithmicStablecoinClient::new(&env, &id);
        let admin = Address::generate(&env);
        let oracle = Address::generate(&env);

        client.init(&admin, &oracle);

        assert_eq!(client.get_admin(), admin);
        assert_eq!(client.get_price(), 10_000_000);
        assert_eq!(client.get_target_price(), 10_000_000);
        assert!(!client.is_paused());
    }

    #[test]
    fn test_mint() {
        let (env, admin, _oracle, user, client) = setup();

        client.mint(&admin, &user, &1000);

        assert_eq!(client.balance(&user), 1000);
        assert_eq!(client.total_supply(), 1000);
    }

    #[test]
    fn test_burn() {
        let (env, admin, _oracle, user, client) = setup();

        client.mint(&admin, &user, &1000);
        client.burn(&user, &500);

        assert_eq!(client.balance(&user), 500);
        assert_eq!(client.total_supply(), 500);
    }

    #[test]
    fn test_transfer() {
        let (env, admin, _oracle, user, client) = setup();
        let recipient = Address::generate(&env);

        client.mint(&admin, &user, &1000);
        client.transfer(&user, &recipient, &300);

        assert_eq!(client.balance(&user), 700);
        assert_eq!(client.balance(&recipient), 300);
    }

    #[test]
    fn test_set_price() {
        let (env, admin, oracle, _user, client) = setup();

        client.set_price(&oracle, &12_000_000);

        assert_eq!(client.get_price(), 12_000_000);
    }

    #[test]
    fn test_rebase_expansion() {
        let (env, admin, oracle, _user, client) = setup();

        client.mint(&admin, &admin, &1_000_000);
        client.set_price(&oracle, &11_000_000);

        env.ledger().set_timestamp(4000);

        let info = client.rebase(&admin);

        assert!(info.new_supply > info.old_supply);
    }

    #[test]
    fn test_pause_and_unpause() {
        let (env, admin, _oracle, user, client) = setup();

        client.pause(&admin);
        assert!(client.is_paused());

        let result = client.try_mint(&admin, &user, &100);
        assert_eq!(result, Err(Ok(Error::ContractPaused)));

        client.unpause(&admin);
        assert!(!client.is_paused());

        client.mint(&admin, &user, &100);
        assert_eq!(client.balance(&user), 100);
    }

    #[test]
    fn test_reserve_operations() {
        let (env, admin, _oracle, _user, client) = setup();

        client.add_reserve(&admin, &5000);
        assert_eq!(client.get_reserve(), 5000);

        client.withdraw_reserve(&admin, &2000);
        assert_eq!(client.get_reserve(), 3000);
    }

    fn setup_psm() -> (
        Env,
        Address,
        Address,
        Address,
        Address,
        AlgorithmicStablecoinClient<'static>,
    ) {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let oracle = Address::generate(&env);
        let user = Address::generate(&env);
        let asset_admin = Address::generate(&env);
        let usdc_contract = env.register_stellar_asset_contract_v2(asset_admin.clone());
        let usdt_contract = env.register_stellar_asset_contract_v2(asset_admin.clone());
        let usdc = usdc_contract.address();
        let usdt = usdt_contract.address();
        StellarAssetClient::new(&env, &usdc).mint(&user, &10_000);
        StellarAssetClient::new(&env, &usdt).mint(&user, &10_000);
        let id = env.register_contract(None, AlgorithmicStablecoin);
        let client = AlgorithmicStablecoinClient::new(&env, &id);
        client.init(&admin, &oracle);
        client.configure_psm(&admin, &usdc, &usdt, &30, &20);

        let env = std::boxed::Box::leak(std::boxed::Box::new(env));
        let client = AlgorithmicStablecoinClient::new(env, &id);
        (env.clone(), admin, user, usdc, usdt, client)
    }

    #[test]
    fn psm_mint_is_one_to_one_less_fee_and_fully_backed() {
        let (env, _admin, user, usdc, _usdt, client) = setup_psm();
        let result = client.psm_mint(&user, &usdc, &1_000, &997);

        assert_eq!(result.fee, 3);
        assert_eq!(result.amount_out, 997);
        assert_eq!(client.balance(&user), 997);
        assert_eq!(client.get_psm_supply(), 997);
        assert_eq!(client.get_collateral_reserve(&usdc), 1_000);
        assert_eq!(TokenClient::new(&env, &usdc).balance(&user), 9_000);
    }

    #[test]
    fn psm_burn_returns_collateral_and_retains_fees() {
        let (env, _admin, user, usdc, _usdt, client) = setup_psm();
        client.psm_mint(&user, &usdc, &1_000, &997);
        let result = client.psm_burn(&user, &usdc, &500, &499);

        assert_eq!(result.fee, 1);
        assert_eq!(result.amount_out, 499);
        assert_eq!(client.balance(&user), 497);
        assert_eq!(client.get_psm_supply(), 497);
        assert_eq!(client.get_collateral_reserve(&usdc), 501);
        assert_eq!(TokenClient::new(&env, &usdc).balance(&user), 9_499);
    }

    #[test]
    fn dynamic_fees_and_slippage_limits_are_enforced() {
        let (_env, admin, user, usdc, _usdt, client) = setup_psm();
        client.set_psm_fees(&admin, &100, &50);
        let config = client.get_psm_config();
        assert_eq!(config.mint_fee_bps, 100);
        assert_eq!(config.burn_fee_bps, 50);
        assert_eq!(
            client.try_psm_mint(&user, &usdc, &1_000, &991),
            Err(Ok(Error::SlippageExceeded))
        );
        assert_eq!(
            client.try_set_psm_fees(&admin, &(MAX_PSM_FEE_BPS + 1), &0),
            Err(Ok(Error::InvalidFee))
        );
    }

    #[test]
    fn rejects_unsupported_collateral_and_insufficient_reserves() {
        let (env, _admin, user, usdc, usdt, client) = setup_psm();
        let unsupported = Address::generate(&env);
        assert_eq!(
            client.try_psm_mint(&user, &unsupported, &100, &0),
            Err(Ok(Error::UnsupportedCollateral))
        );
        client.psm_mint(&user, &usdc, &1_000, &0);
        assert_eq!(
            client.try_psm_burn(&user, &usdt, &100, &0),
            Err(Ok(Error::InsufficientReserve))
        );
    }

    #[test]
    fn psm_configuration_is_single_use_and_pause_applies_to_swaps() {
        let (_env, admin, user, usdc, usdt, client) = setup_psm();
        assert_eq!(
            client.try_configure_psm(&admin, &usdc, &usdt, &0, &0),
            Err(Ok(Error::PsmAlreadyConfigured))
        );
        client.pause(&admin);
        assert_eq!(
            client.try_psm_mint(&user, &usdc, &100, &0),
            Err(Ok(Error::ContractPaused))
        );
    }
}
