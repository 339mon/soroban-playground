// Copyright (c) 2026 StellarDevTools
// SPDX-License-Identifier: MIT

#[cfg(test)]
mod tests {
    use soroban_sdk::{testutils::Address as _, Address, Env, String};

    use crate::{NftFractionalVault, NftFractionalVaultClient};

    #[test]
    fn test_initialize() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(NftFractionalVault, ());
        let client = NftFractionalVaultClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        client.initialize(&admin);
    }

    #[test]
    fn test_create_vault() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(NftFractionalVault, ());
        let client = NftFractionalVaultClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let creator = Address::generate(&env);
        let nft_contract = Address::generate(&env);

        client.initialize(&admin);
        let vault_id = client.create_vault(
            &creator,
            &nft_contract,
            &1_u32,
            &String::from_str(&env, "TEST-FRAC"),
            &1_000_i128,
            &1_000_000_i128,
        );
        assert_eq!(vault_id, 0);
        assert_eq!(client.balance_of(&vault_id, &creator), 1_000);
    }

    #[test]
    fn test_transfer_fractions() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(NftFractionalVault, ());
        let client = NftFractionalVaultClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let creator = Address::generate(&env);
        let buyer = Address::generate(&env);
        let nft_contract = Address::generate(&env);

        client.initialize(&admin);
        let vault_id = client.create_vault(
            &creator,
            &nft_contract,
            &1_u32,
            &String::from_str(&env, "TEST-FRAC"),
            &1_000_i128,
            &1_000_000_i128,
        );

        client.transfer(&vault_id, &creator, &buyer, &100_i128);
        assert_eq!(client.balance_of(&vault_id, &creator), 900);
        assert_eq!(client.balance_of(&vault_id, &buyer), 100);
    }
}
