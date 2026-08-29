// contracts/subscription/src/lib.rs
use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, Symbol};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Subscription {
    pub subscriber: Address,
    pub merchant: Address,
    pub allowance_per_period: i128,
    pub period_duration: u64, // in seconds
    pub next_billing_time: u64,
    pub end_time: u64,
    pub active: bool,
}

#[contracttype]
pub enum DataKey {
    Sub(Address, Address), // (subscriber, merchant)
}

#[contract]
pub struct SubscriptionBillingContract;

#[contractimpl]
impl SubscriptionBillingContract {
    pub fn subscribe(
        env: Env,
        subscriber: Address,
        merchant: Address,
        allowance_per_period: i128,
        period_duration: u64,
        duration: u64,
    ) {
        subscriber.require_auth();
        if allowance_per_period <= 0 {
            panic!("Allowance must be positive");
        }

        let current_time = env.ledger().timestamp();
        let end_time = current_time + duration;
        let next_billing_time = current_time;

        let key = DataKey::Sub(subscriber.clone(), merchant.clone());
        let sub = Subscription {
            subscriber: subscriber.clone(),
            merchant: merchant.clone(),
            allowance_per_period,
            period_duration,
            next_billing_time,
            end_time,
            active: true,
        };

        env.storage().persistent().set(&key, &sub);
        env.events().publish(
            (Symbol::new(&env, "Subscribed"), subscriber),
            (merchant, allowance_per_period),
        );
    }

    pub fn pull_payment(env: Env, merchant: Address, subscriber: Address) {
        merchant.require_auth();

        let key = DataKey::Sub(subscriber.clone(), merchant.clone());
        let mut sub: Subscription = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| panic!("Subscription not found"));

        if !sub.active {
            panic!("Subscription is not active");
        }

        let current_time = env.ledger().timestamp();
        if current_time < sub.next_billing_time {
            panic!("Billing period has not yet arrived");
        }

        if current_time > sub.end_time {
            sub.active = false;
            env.storage().persistent().set(&key, &sub);
            panic!("Subscription has expired");
        }

        sub.next_billing_time += sub.period_duration;
        env.storage().persistent().set(&key, &sub);

        env.events().publish(
            (Symbol::new(&env, "PaymentPulled"), subscriber),
            (merchant, sub.allowance_per_period),
        );
    }

    pub fn cancel_subscription(env: Env, subscriber: Address, merchant: Address) {
        subscriber.require_auth();

        let key = DataKey::Sub(subscriber.clone(), merchant.clone());
        let mut sub: Subscription = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| panic!("Subscription not found"));

        sub.active = false;
        env.storage().persistent().set(&key, &sub);

        env.events().publish(
            (Symbol::new(&env, "SubscriptionCancelled"), subscriber),
            merchant,
        );
    }
}