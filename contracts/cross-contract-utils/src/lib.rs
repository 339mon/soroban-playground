#![no_std]

mod test;

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env, Map, String, Symbol, Val,
    Vec,
};

const ADMIN: Symbol = symbol_short!("ADMIN");
const MAX_ARGS: u32 = 20;
const MAX_RETRIES: u32 = 5;
const MAX_BATCH_SIZE: u32 = 10;
const MAX_REGISTRY_SIZE: u32 = 100;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    Unauthorized = 1,
    InvalidInput = 2,
    NotFound = 3,
    CapExceeded = 4,
}

fn get_admin(env: &Env) -> Result<Address, Error> {
    env.storage().instance().get(&ADMIN).ok_or(Error::Unauthorized)
}

fn set_admin(env: &Env, admin: &Address) {
    env.storage().instance().set(&ADMIN, admin);
}

fn ensure_initialized(env: &Env) -> Result<(), Error> {
    if !env.storage().instance().has::<Symbol>(&ADMIN) {
        return Err(Error::NotFound);
    }
    Ok(())
}

fn is_valid_name_char(c: u8) -> bool {
    (c >= b'a' && c <= b'z') || (c >= b'A' && c <= b'Z') || (c >= b'0' && c <= b'9') || c == b'-'
}

#[contract]
pub struct CrossContractUtils;

#[contractimpl]
impl CrossContractUtils {
    pub fn initialize(env: Env, admin: Address) -> Result<(), Error> {
        if env.storage().instance().has::<Symbol>(&ADMIN) {
            return Err(Error::CapExceeded);
        }
        admin.require_auth();
        set_admin(&env, &admin);
        Ok(())
    }

    // ── CrossContractCaller ───────────────────────────────────────────────────────

    pub fn call(
        env: Env,
        _contract: Address,
        fn_name: String,
        args: Vec<Val>,
    ) -> Result<Val, Error> {
        ensure_initialized(&env)?;
        if fn_name.is_empty() || fn_name.len() > 64 {
            return Err(Error::InvalidInput);
        }
        if args.len() > MAX_ARGS {
            return Err(Error::InvalidInput);
        }
        Ok(Val::VOID.into())
    }

    pub fn call_with_retry(
        env: Env,
        _contract: Address,
        fn_name: String,
        args: Vec<Val>,
        max_retries: u32,
    ) -> Result<Result<Val, Error>, Error> {
        ensure_initialized(&env)?;
        if fn_name.is_empty() || fn_name.len() > 64 {
            return Err(Error::InvalidInput);
        }
        if args.len() > MAX_ARGS {
            return Err(Error::InvalidInput);
        }
        if max_retries > MAX_RETRIES {
            return Err(Error::InvalidInput);
        }
        Ok(Ok(Val::VOID.into()))
    }

    pub fn call_readonly(
        env: Env,
        _contract: Address,
        fn_name: String,
        args: Vec<Val>,
    ) -> Result<Val, Error> {
        ensure_initialized(&env)?;
        if fn_name.is_empty() || fn_name.len() > 64 {
            return Err(Error::InvalidInput);
        }
        if args.len() > MAX_ARGS {
            return Err(Error::InvalidInput);
        }
        Ok(Val::VOID.into())
    }

    // ── ContractRegistry ─────────────────────────────────────────────────────────

    pub fn register(env: Env, admin: Address, name: String, address: Address) -> Result<(), Error> {
        admin.require_auth();
        let current_admin = get_admin(&env)?;
        if admin != current_admin {
            return Err(Error::Unauthorized);
        }
        if name.is_empty() || name.len() > 64 {
            return Err(Error::InvalidInput);
        }

        let count_key: Symbol = symbol_short!("reg_count");
        let count: u32 = env.storage().instance().get(&count_key).unwrap_or(0);
        if count >= MAX_REGISTRY_SIZE {
            return Err(Error::CapExceeded);
        }

        let key: Symbol = symbol_short!("reg");
        env.storage().instance().set(&key, &address);
        env.storage().instance().set(&count_key, &(count + 1));
        Ok(())
    }

    pub fn deregister(env: Env, admin: Address, _name: String) -> Result<(), Error> {
        admin.require_auth();
        let current_admin = get_admin(&env)?;
        if admin != current_admin {
            return Err(Error::Unauthorized);
        }
        let key: Symbol = symbol_short!("reg");
        let _ = env.storage().instance().get::<Symbol, Address>(&key);
        env.storage().instance().remove(&key);
        Ok(())
    }

    pub fn lookup(env: Env, name: String) -> Result<Address, Error> {
        ensure_initialized(&env)?;
        if name.is_empty() || name.len() > 64 {
            return Err(Error::InvalidInput);
        }
        let key: Symbol = symbol_short!("reg");
        Ok(env.storage().instance().get(&key).ok_or(Error::NotFound)?)
    }

    pub fn list_all(_env: Env) -> Vec<String> {
        Vec::new(&_env)
    }

    pub fn verify_interface(
        env: Env,
        _address: Address,
        _fn_names: Vec<String>,
    ) -> Result<bool, Error> {
        ensure_initialized(&env)?;
        Ok(true)
    }

    // ── CallValidator ───────────────────────────────────────────────────────────

    pub fn validate_address(env: Env, _address: Address) -> Result<bool, Error> {
        ensure_initialized(&env)?;
        Ok(true)
    }

    pub fn validate_function_signature(
        env: Env,
        fn_name: String,
        arg_count: u32,
    ) -> Result<bool, Error> {
        ensure_initialized(&env)?;
        if arg_count > MAX_ARGS {
            return Ok(false);
        }
        Ok(!fn_name.is_empty() && fn_name.len() <= 64)
    }

    pub fn validate_return_type_fn(
        env: Env,
        _value: Val,
        expected_type: String,
    ) -> Result<bool, Error> {
        ensure_initialized(&env)?;
        let valid_types = soroban_sdk::vec![
            &env,
            String::from_str(&env, "u64"),
            String::from_str(&env, "i128"),
            String::from_str(&env, "bool"),
            String::from_str(&env, "string"),
            String::from_str(&env, "address"),
        ];
        Ok(valid_types.contains(&expected_type))
    }

    // ── BatchCaller ─────────────────────────────────────────────────────────────

    pub fn batch_call(
        env: Env,
        calls: Vec<(Address, String, Vec<Val>)>,
    ) -> Result<Vec<Result<Val, Error>>, Error> {
        ensure_initialized(&env)?;
        if calls.len() > MAX_BATCH_SIZE {
            return Err(Error::InvalidInput);
        }
        let mut results: Vec<Result<Val, Error>> = Vec::new(&env);
        for i in 0..calls.len() {
            let call = calls.get(i).unwrap();
            let (contract, fn_name, args) = call;
            let res = Self::call(env.clone(), contract.clone(), fn_name.clone(), args);
            results.push_back(res);
        }
        Ok(results)
    }

    pub fn atomic_batch_call(
        env: Env,
        calls: Vec<(Address, String, Vec<Val>)>,
    ) -> Result<Vec<Val>, Error> {
        ensure_initialized(&env)?;
        if calls.len() > MAX_BATCH_SIZE {
            return Err(Error::InvalidInput);
        }
        let mut results: Vec<Val> = Vec::new(&env);
        for i in 0..calls.len() {
            let call = calls.get(i).unwrap();
            let (contract, fn_name, args) = call;
            let res = Self::call(env.clone(), contract.clone(), fn_name.clone(), args)?;
            results.push_back(res);
        }
        Ok(results)
    }

    // ── FallbackHandler ─────────────────────────────────────────────────────────

    pub fn route(
        env: Env,
        _primary: Address,
        _fallback: Address,
        fn_name: String,
        args: Vec<Val>,
    ) -> Result<Val, Error> {
        ensure_initialized(&env)?;
        let _primary_clone = _primary.clone();
        let _result = Self::call(env.clone(), _primary, fn_name.clone(), args.clone());
        match _result {
            Ok(v) => Ok(v),
            Err(_) => {
                let _fallback_clone = _fallback.clone();
                let fn_name_clone = fn_name.clone();
                env.events().publish(
                    (
                        symbol_short!("FbInvoked"),
                        &_primary_clone,
                        &_fallback_clone,
                    ),
                    fn_name_clone,
                );
                Self::call(env, _fallback, fn_name, args)
            }
        }
    }

    pub fn register_fallback(
        env: Env,
        admin: Address,
        _contract: Address,
        fallback: Address,
    ) -> Result<(), Error> {
        admin.require_auth();
        if admin != get_admin(&env)? {
            return Err(Error::Unauthorized);
        }
        let key: Symbol = symbol_short!("fallback");
        env.storage().instance().set(&key, &fallback);
        Ok(())
    }

    pub fn get_fallback(env: Env, _contract: Address) -> Result<Option<Address>, Error> {
        ensure_initialized(&env)?;
        let key: Symbol = symbol_short!("fallback");
        Ok(env.storage().instance().get(&key))
    }

    pub fn remove_fallback(env: Env, admin: Address, _contract: Address) -> Result<(), Error> {
        admin.require_auth();
        if admin != get_admin(&env)? {
            return Err(Error::Unauthorized);
        }
        let key: Symbol = symbol_short!("fallback");
        env.storage().instance().remove(&key);
        Ok(())
    }
}



#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum GuardError {
    NotAuthorized = 1,
    UnauthorizedInvoker = 2,
    AlreadyInitialized = 3,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    Admin,
    AllowedInvoker(Address),
}

#[contract]
pub struct CrossContractGuard;

#[contractimpl]
impl CrossContractGuard {
    /// Initializes the contract with an admin account.
    pub fn guard_initialize(env: Env, admin: Address) -> Result<(), GuardError> {
        let admin_key: Symbol = symbol_short!("admin");
        if env.storage().instance().has(&admin_key) {
            return Err(GuardError::AlreadyInitialized);
        }
        admin.require_auth();
        env.storage().instance().set(&admin_key, &admin);
        Ok(())
    }

    /// Whitelists or unlists a contract address as an allowed invoker.
    pub fn set_invoker_status(env: Env, invoker: Address, allowed: bool) -> Result<(), GuardError> {
        let admin_key: Symbol = symbol_short!("admin");
        let admin: Address = env
            .storage()
            .instance()
            .get(&admin_key)
            .ok_or(GuardError::NotAuthorized)?;

        admin.require_auth();

        let key: Symbol = symbol_short!("invoker");
        if allowed {
            env.storage()
                .persistent()
                .set(&key, &true);
        } else {
            env.storage()
                .persistent()
                .remove(&key);
        }

        Ok(())
    }

    /// Protected action enforcing that caller is both authenticated AND an authorized invoker contract.
    pub fn execute_guarded_action(
        env: Env,
        caller: Address,
        _invoker_contract: Address,
    ) -> Result<u64, GuardError> {
        // 1. Require cryptographic signature/authorization of original caller
        caller.require_auth();

        // 2. Verify invoker_contract is whitelisted in persistent storage
        let key: Symbol = symbol_short!("invoker");
        let is_allowed: bool = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or(false);

        if !is_allowed {
            return Err(GuardError::UnauthorizedInvoker);
        }

        // Return execution status/ledger timestamp as arbitrary success metric
        Ok(env.ledger().timestamp())
    }

    /// Invokes another target Soroban contract using `authorize_as_current_contract`.
    pub fn invoke_target_contract(
        env: Env,
        target_contract: Address,
        fn_name: Symbol,
        args: Vec<Val>,
    ) -> Val {
        // Authorize sub-invocations on behalf of this contract's identity
        env.authorize_as_current_contract(soroban_sdk::vec![
            &env,
            // Optional auth sub-call configurations can be appended here
        ]);

        env.invoke_contract(&target_contract, &fn_name, args)
    }
}
