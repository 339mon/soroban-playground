use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

struct Setup {
    env: Env,
    client: StakingDerivativesClient<'static>,
    admin: Address,
    alice: Address,
    bob: Address,
    token: Address,
}

fn setup() -> Setup {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let asset = env.register_stellar_asset_contract_v2(token_admin.clone());
    let token = asset.address();
    let sac = StellarAssetClient::new(&env, &token);
    sac.mint(&admin, &10_000);
    sac.mint(&alice, &10_000);
    sac.mint(&bob, &10_000);
    let id = env.register_contract(None, StakingDerivatives);
    let client = StakingDerivativesClient::new(&env, &id);
    client.initialize(&admin, &token, &100);
    Setup {
        env,
        client,
        admin,
        alice,
        bob,
        token,
    }
}

#[test]
fn rewards_accrue_into_exchange_rate_and_later_deposits_get_fewer_shares() {
    let s = setup();
    assert_eq!(s.client.deposit(&s.alice, &1_000), 1_000);
    assert_eq!(s.client.accrue_rewards(&s.admin, &200), 12_000_000);
    assert_eq!(s.client.exchange_rate(), 12_000_000);
    assert_eq!(s.client.convert_to_assets(&500), 600);
    assert_eq!(s.client.deposit(&s.bob, &600), 500);
    assert_eq!(s.client.totals(), (1_800, 1_500, 0, 200));
}

#[test]
fn unbonding_reserves_value_and_claims_only_after_maturity() {
    let s = setup();
    let token = TokenClient::new(&s.env, &s.token);
    s.client.deposit(&s.alice, &1_000);
    s.client.accrue_rewards(&s.admin, &200);

    let id = s.client.request_unstake(&s.alice, &500);
    let request = s.client.get_request(&id);
    assert_eq!(request.amount, 600);
    assert_eq!(s.client.totals(), (600, 500, 600, 200));
    assert_eq!(
        s.client.try_claim_unstake(&s.alice, &id),
        Err(Ok(Error::RequestNotReady))
    );

    s.env.ledger().with_mut(|ledger| ledger.timestamp += 100);
    assert_eq!(s.client.claim_unstake(&s.alice, &id), 600);
    assert_eq!(token.balance(&s.alice), 9_600);
    assert_eq!(s.client.totals(), (600, 500, 0, 200));
    assert_eq!(
        s.client.try_claim_unstake(&s.alice, &id),
        Err(Ok(Error::AlreadyClaimed))
    );
}

#[test]
fn full_exit_does_not_leave_rounding_dust() {
    let s = setup();
    s.client.deposit(&s.alice, &3);
    s.client.accrue_rewards(&s.admin, &1);
    let id = s.client.request_unstake(&s.alice, &3);
    assert_eq!(s.client.get_request(&id).amount, 4);
    assert_eq!(s.client.totals(), (0, 0, 4, 1));
    assert_eq!(s.client.exchange_rate(), RATE_SCALE);
}

#[test]
fn queue_entries_are_owned_and_independent() {
    let s = setup();
    s.client.deposit(&s.alice, &500);
    s.client.deposit(&s.bob, &500);
    let first = s.client.request_unstake(&s.alice, &100);
    let second = s.client.request_unstake(&s.bob, &200);
    assert_eq!(first, 0);
    assert_eq!(second, 1);
    assert_eq!(
        s.client.try_claim_unstake(&s.bob, &first),
        Err(Ok(Error::RequestNotFound))
    );
}

#[test]
fn validates_initialization_amounts_balances_and_empty_rewards() {
    let s = setup();
    assert_eq!(
        s.client.try_deposit(&s.alice, &0),
        Err(Ok(Error::InvalidAmount))
    );
    assert_eq!(
        s.client.try_accrue_rewards(&s.admin, &10),
        Err(Ok(Error::NoActiveStake))
    );
    s.client.deposit(&s.alice, &100);
    assert_eq!(
        s.client.try_request_unstake(&s.alice, &101),
        Err(Ok(Error::InsufficientShares))
    );

    let id = s.env.register_contract(None, StakingDerivatives);
    let other = StakingDerivativesClient::new(&s.env, &id);
    assert_eq!(
        other.try_initialize(&s.admin, &s.token, &0),
        Err(Ok(Error::InvalidUnbondingPeriod))
    );
}
