#![cfg(test)]

use super::*;
use soroban_sdk::{contract, contractimpl, contracttype, testutils::Address as _, Address, Env};

// ─── Mock Token ────────────────────────────────────────────────────

#[contract]
pub struct MockToken;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MockDataKey {
    Balances(Address),
}

#[contractimpl]
impl MockToken {
    pub fn mint(env: Env, to: Address, amount: i128) {
        let key = MockDataKey::Balances(to.clone());
        let current: i128 = env.storage().instance().get(&key).unwrap_or(0);
        env.storage().instance().set(&key, &(current + amount));
    }

    pub fn balance(env: Env, id: Address) -> i128 {
        let key = MockDataKey::Balances(id);
        env.storage().instance().get(&key).unwrap_or(0)
    }

    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        from.require_auth();
        let from_key = MockDataKey::Balances(from.clone());
        let from_bal: i128 = env.storage().instance().get(&from_key).unwrap_or(0);
        assert!(from_bal >= amount, "Insufficient balance");
        env.storage()
            .instance()
            .set(&from_key, &(from_bal - amount));

        let to_key = MockDataKey::Balances(to.clone());
        let to_bal: i128 = env.storage().instance().get(&to_key).unwrap_or(0);
        env.storage().instance().set(&to_key, &(to_bal + amount));
    }
}

// ─── Helpers ───────────────────────────────────────────────────────

fn setup_env() -> (Env, Address, AmmClient<'static>, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);

    let amm_id = env.register(Amm, ());
    let amm_client = AmmClient::new(&env, &amm_id);

    let token_a_id = env.register(MockToken, ());
    let token_b_id = env.register(MockToken, ());

    amm_client.init_contract(&admin);

    (env, admin, amm_client, token_a_id, token_b_id)
}

fn mint_tokens(env: &Env, token_id: &Address, to: &Address, amount: i128) {
    let client = MockTokenClient::new(env, token_id);
    client.mint(to, &amount);
}

// ─── Tests ─────────────────────────────────────────────────────────

#[test]
fn test_create_pool() {
    let (_env, admin, amm, token_a, token_b) = setup_env();

    let pool_id = amm.create_pool(&admin, &token_a, &token_b, &30);
    assert_eq!(pool_id, 0);

    let pool = amm.get_pool(&pool_id);
    assert_eq!(pool.token_a, token_a);
    assert_eq!(pool.token_b, token_b);
    assert_eq!(pool.fee_bps, 30);
    assert_eq!(pool.reserve_a, 0);
    assert_eq!(pool.reserve_b, 0);
}

#[test]
#[should_panic(expected = "Token addresses must differ")]
fn test_create_pool_same_tokens() {
    let (_env, admin, amm, token_a, _) = setup_env();
    amm.create_pool(&admin, &token_a, &token_a, &30);
}

#[test]
#[should_panic(expected = "Fee cannot exceed 10%")]
fn test_create_pool_excessive_fee() {
    let (_env, admin, amm, token_a, token_b) = setup_env();
    amm.create_pool(&admin, &token_a, &token_b, &1001);
}

#[test]
fn test_add_liquidity_initial() {
    let (env, admin, amm, token_a, token_b) = setup_env();

    let provider = Address::generate(&env);
    let pool_id = amm.create_pool(&admin, &token_a, &token_b, &30);

    // Mint tokens to provider.
    mint_tokens(&env, &token_a, &provider, 10_000);
    mint_tokens(&env, &token_b, &provider, 20_000);

    let lp = amm.add_liquidity(&provider, &pool_id, &10_000, &20_000);
    assert!(lp > 0);

    let pool = amm.get_pool(&pool_id);
    assert_eq!(pool.reserve_a, 10_000);
    assert_eq!(pool.reserve_b, 20_000);
    assert_eq!(pool.lp_total_supply, lp);

    let balance = amm.get_lp_balance(&pool_id, &provider);
    assert_eq!(balance, lp);
}

#[test]
fn test_add_liquidity_subsequent() {
    let (env, admin, amm, token_a, token_b) = setup_env();

    let provider1 = Address::generate(&env);
    let provider2 = Address::generate(&env);
    let pool_id = amm.create_pool(&admin, &token_a, &token_b, &30);

    // First provider.
    mint_tokens(&env, &token_a, &provider1, 10_000);
    mint_tokens(&env, &token_b, &provider1, 10_000);
    let lp1 = amm.add_liquidity(&provider1, &pool_id, &10_000, &10_000);

    // Second provider.
    mint_tokens(&env, &token_a, &provider2, 5_000);
    mint_tokens(&env, &token_b, &provider2, 5_000);
    let lp2 = amm.add_liquidity(&provider2, &pool_id, &5_000, &5_000);

    // Second provider should get half the LP tokens of first.
    assert_eq!(lp2, lp1 / 2);
}

#[test]
fn test_remove_liquidity() {
    let (env, admin, amm, token_a, token_b) = setup_env();

    let provider = Address::generate(&env);
    let pool_id = amm.create_pool(&admin, &token_a, &token_b, &30);

    mint_tokens(&env, &token_a, &provider, 10_000);
    mint_tokens(&env, &token_b, &provider, 10_000);

    let lp = amm.add_liquidity(&provider, &pool_id, &10_000, &10_000);

    // Withdraw half.
    let (out_a, out_b) = amm.remove_liquidity(&provider, &pool_id, &(lp / 2));
    assert_eq!(out_a, 5_000);
    assert_eq!(out_b, 5_000);

    let pool = amm.get_pool(&pool_id);
    assert_eq!(pool.reserve_a, 5_000);
    assert_eq!(pool.reserve_b, 5_000);
}

#[test]
fn test_swap() {
    let (env, admin, amm, token_a, token_b) = setup_env();

    let provider = Address::generate(&env);
    let user = Address::generate(&env);
    let pool_id = amm.create_pool(&admin, &token_a, &token_b, &30);

    // Add liquidity.
    mint_tokens(&env, &token_a, &provider, 100_000);
    mint_tokens(&env, &token_b, &provider, 100_000);
    amm.add_liquidity(&provider, &pool_id, &100_000, &100_000);

    // User swaps token_a for token_b.
    mint_tokens(&env, &token_a, &user, 1_000);
    let out = amm.swap(&user, &pool_id, &token_a, &1_000, &0);

    // With 0.3% fee, output should be slightly less than ~990 (constant product).
    assert!(out > 0);
    assert!(out < 1_000); // Must be less due to price impact + fee.

    // Check user received tokens.
    let user_b = MockTokenClient::new(&env, &token_b).balance(&user);
    assert_eq!(user_b, out);
}

#[test]
#[should_panic(expected = "Slippage tolerance exceeded")]
fn test_swap_slippage_protection() {
    let (env, admin, amm, token_a, token_b) = setup_env();

    let provider = Address::generate(&env);
    let user = Address::generate(&env);
    let pool_id = amm.create_pool(&admin, &token_a, &token_b, &30);

    mint_tokens(&env, &token_a, &provider, 100_000);
    mint_tokens(&env, &token_b, &provider, 100_000);
    amm.add_liquidity(&provider, &pool_id, &100_000, &100_000);

    mint_tokens(&env, &token_a, &user, 1_000);
    // Set min_amount_out unrealistically high.
    amm.swap(&user, &pool_id, &token_a, &1_000, &999);
}

#[test]
fn test_get_price() {
    let (env, admin, amm, token_a, token_b) = setup_env();

    let provider = Address::generate(&env);
    let pool_id = amm.create_pool(&admin, &token_a, &token_b, &30);

    mint_tokens(&env, &token_a, &provider, 10_000);
    mint_tokens(&env, &token_b, &provider, 20_000);
    amm.add_liquidity(&provider, &pool_id, &10_000, &20_000);

    // Price of token_a in terms of token_b: 20000/10000 = 2.0 scaled by 1e6 = 2_000_000.
    let price_a = amm.get_price(&pool_id, &token_a);
    assert_eq!(price_a, 2_000_000);

    // Price of token_b in terms of token_a: 10000/20000 = 0.5 scaled by 1e6 = 500_000.
    let price_b = amm.get_price(&pool_id, &token_b);
    assert_eq!(price_b, 500_000);
}

#[test]
fn test_fee_distribution_via_reserves() {
    let (env, admin, amm, token_a, token_b) = setup_env();

    let provider = Address::generate(&env);
    let user = Address::generate(&env);
    let pool_id = amm.create_pool(&admin, &token_a, &token_b, &30);

    // Add liquidity.
    mint_tokens(&env, &token_a, &provider, 100_000);
    mint_tokens(&env, &token_b, &provider, 100_000);
    let lp = amm.add_liquidity(&provider, &pool_id, &100_000, &100_000);

    // Execute a swap (fees stay in pool reserves).
    mint_tokens(&env, &token_a, &user, 10_000);
    amm.swap(&user, &pool_id, &token_a, &10_000, &0);

    // Withdraw all liquidity: provider should get back more value than deposited
    // because swap fees accumulated in the reserves.
    let (out_a, out_b) = amm.remove_liquidity(&provider, &pool_id, &lp);

    // Provider deposited 100k of each. After swaps, total value of reserves increased.
    // out_a should be > 100_000 (received swap input) and out_b < 100_000 (paid swap output).
    // But total value = out_a + out_b should be > 200_000 due to fees.
    assert!(out_a > 100_000);
    assert!(out_b < 100_000);
    assert!(out_a + out_b > 200_000); // Fee profit.
}

#[test]
fn test_multiple_pools() {
    let (env, admin, amm, token_a, token_b) = setup_env();

    let token_c_id = env.register(MockToken, ());

    let pool_0 = amm.create_pool(&admin, &token_a, &token_b, &30);
    let pool_1 = amm.create_pool(&admin, &token_a, &token_c_id, &50);

    assert_eq!(pool_0, 0);
    assert_eq!(pool_1, 1);

    let info_0 = amm.get_pool(&pool_0);
    let info_1 = amm.get_pool(&pool_1);

    assert_eq!(info_0.fee_bps, 30);
    assert_eq!(info_1.fee_bps, 50);
}

// Issue #215: Tests for cache invalidation
#[test]
fn test_cache_invalidation_on_add_liquidity() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let amm_id = env.register_contract(None, Amm);
    let amm = AmmClient::new(&env, &amm_id);

    amm.init_contract(&admin);

    let token_a = Address::generate(&env);
    let token_b = Address::generate(&env);

    let pool_id = amm.create_pool(&admin, &token_a, &token_b, &30);

    let provider = Address::generate(&env);
    
    // Add liquidity - should trigger cache invalidation
    let lp_tokens = amm.add_liquidity(&provider, &pool_id, &1000, &2000);
    assert!(lp_tokens > 0);

    // Verify cache was invalidated by checking that subsequent reads get fresh data
    let pool = amm.get_pool(&pool_id);
    assert_eq!(pool.reserve_a, 1000);
    assert_eq!(pool.reserve_b, 2000);
}

#[test]
fn test_cache_invalidation_on_remove_liquidity() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let amm_id = env.register_contract(None, Amm);
    let amm = AmmClient::new(&env, &amm_id);

    amm.init_contract(&admin);

    let token_a = Address::generate(&env);
    let token_b = Address::generate(&env);

    let pool_id = amm.create_pool(&admin, &token_a, &token_b, &30);

    let provider = Address::generate(&env);
    amm.add_liquidity(&provider, &pool_id, &1000, &2000);

    // Remove liquidity - should trigger cache invalidation
    let (amount_a, amount_b) = amm.remove_liquidity(&provider, &pool_id, &500);
    assert!(amount_a > 0);
    assert!(amount_b > 0);

    // Verify cache was invalidated and data is fresh
    let pool = amm.get_pool(&pool_id);
    assert!(pool.reserve_a < 1000);
    assert!(pool.reserve_b < 2000);
}

#[test]
fn test_cache_invalidation_on_swap() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let amm_id = env.register_contract(None, Amm);
    let amm = AmmClient::new(&env, &amm_id);

    amm.init_contract(&admin);

    let token_a = Address::generate(&env);
    let token_b = Address::generate(&env);

    let pool_id = amm.create_pool(&admin, &token_a, &token_b, &30);

    let provider = Address::generate(&env);
    amm.add_liquidity(&provider, &pool_id, &1000, &2000);

    // Execute swap - should trigger cache invalidation
    let user = Address::generate(&env);
    let amount_out = amm.swap(&user, &pool_id, &token_a, &100, &1);
    assert!(amount_out > 0);

    // Verify cache was invalidated and reserves updated
    let pool = amm.get_pool(&pool_id);
    assert!(pool.reserve_a > 1000); // Increased by 100
    assert!(pool.reserve_b < 2000); // Decreased by amount_out
}
