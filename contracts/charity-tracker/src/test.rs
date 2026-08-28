#![cfg(test)]
use super::*;
use soroban_sdk::{testutils::Address as _, Address, Env, String};

fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let token = Address::generate(&env);
    (env, admin, token)
}

#[test]
fn test_initialize() {
    let (env, admin, token) = setup();
    CharityTrackerContract::initialize(env.clone(), admin.clone(), token).unwrap();
    let stored: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
    assert_eq!(stored, admin);
}

#[test]
fn test_create_campaign() {
    let (env, admin, token) = setup();
    CharityTrackerContract::initialize(env.clone(), admin, token).unwrap();
    let id = CharityTrackerContract::create_campaign(
        env.clone(),
        Address::generate(&env),
        String::from_str(&env, "Clean Water"),
        String::from_str(&env, "Provide clean water"),
        100_000,
        3,
    )
    .unwrap();
    assert_eq!(id, 1);
}

#[test]
fn test_donate() {
    let (env, admin, token) = setup();
    CharityTrackerContract::initialize(env.clone(), admin, token).unwrap();
    let id = CharityTrackerContract::create_campaign(
        env.clone(),
        Address::generate(&env),
        String::from_str(&env, "Fund"),
        String::from_str(&env, "Desc"),
        100_000,
        2,
    )
    .unwrap();
    let donation_id =
        CharityTrackerContract::donate(env.clone(), Address::generate(&env), id, 50_000).unwrap();
    assert_eq!(donation_id, 1);
    let campaign = CharityTrackerContract::get_campaign(env, id).unwrap();
    assert_eq!(campaign.raised_amount, 50_000);
}

#[test]
fn test_milestone_flow() {
    let (env, admin, token) = setup();
    CharityTrackerContract::initialize(env.clone(), admin.clone(), token).unwrap();
    let organizer = Address::generate(&env);
    let id = CharityTrackerContract::create_campaign(
        env.clone(),
        organizer.clone(),
        String::from_str(&env, "Fund"),
        String::from_str(&env, "Desc"),
        100_000,
        1,
    )
    .unwrap();
    CharityTrackerContract::add_milestone(
        env.clone(),
        organizer.clone(),
        id,
        1,
        String::from_str(&env, "Build well"),
        50_000,
    )
    .unwrap();
    CharityTrackerContract::complete_milestone(
        env.clone(),
        organizer,
        id,
        1,
        String::from_str(&env, "proof_hash"),
    )
    .unwrap();
    CharityTrackerContract::verify_milestone(env, admin, id, 1).unwrap();
    let m = CharityTrackerContract::get_milestone(env, id, 1).unwrap();
    assert!(m.verified);
}
