#![no_std]

mod storage;
mod types;

#[cfg(test)]
mod test;

use soroban_sdk::{contract, contractimpl, token, Address, Env, Symbol};

use storage::*;
use types::*;

#[contract]
pub struct Amm;

#[contractimpl]
impl Amm {
    /// Initialize the AMM contract with an admin.
    pub fn init_contract(env: Env, admin: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic!("Contract already initialized");
        }
        admin.require_auth();
        set_admin(&env, &admin);
        set_pool_counter(&env, 0);
    }

    /// Create a new liquidity pool for a token pair.
    /// `fee_bps` is the swap fee in basis points (max 1000 = 10%).
    pub fn create_pool(
        env: Env,
        admin: Address,
        token_a: Address,
        token_b: Address,
        fee_bps: u32,
    ) -> u64 {
        admin.require_auth();
        require_admin(&env, &admin);

        assert!(token_a != token_b, "Token addresses must differ");
        assert!(fee_bps <= 1000, "Fee cannot exceed 10%");

        let pool_id = get_pool_counter(&env);
        let pool = Pool {
            pool_id,
            token_a,
            token_b,
            reserve_a: 0,
            reserve_b: 0,
            lp_total_supply: 0,
            fee_bps,
        };

        set_pool(&env, &pool);
        set_pool_counter(&env, pool_id + 1);

        env.events().publish(
            (Symbol::new(&env, "PoolCreated"),),
            (pool_id, &pool.token_a, &pool.token_b, fee_bps),
        );

        pool_id
    }

    /// Add liquidity to a pool. Returns the number of LP tokens minted.
    pub fn add_liquidity(
        env: Env,
        provider: Address,
        pool_id: u64,
        amount_a: i128,
        amount_b: i128,
    ) -> i128 {
        provider.require_auth();

        assert!(amount_a > 0 && amount_b > 0, "Amounts must be positive");

        let mut pool = get_pool(&env, pool_id);

        // Calculate LP tokens to mint.
        let lp_minted = if pool.lp_total_supply == 0 {
            // First deposit: LP tokens = sqrt(amount_a * amount_b).
            isqrt(amount_a * amount_b)
        } else {
            // Subsequent deposits: mint proportional to existing reserves.
            let lp_a = (amount_a * pool.lp_total_supply) / pool.reserve_a;
            let lp_b = (amount_b * pool.lp_total_supply) / pool.reserve_b;
            // Take the minimum to avoid diluting existing LPs.
            if lp_a < lp_b {
                lp_a
            } else {
                lp_b
            }
        };

        assert!(lp_minted > 0, "Insufficient liquidity minted");

        // Transfer tokens from provider to the contract.
        let contract_addr = env.current_contract_address();
        let token_a_client = token::Client::new(&env, &pool.token_a);
        let token_b_client = token::Client::new(&env, &pool.token_b);

        token_a_client.transfer(&provider, &contract_addr, &amount_a);
        token_b_client.transfer(&provider, &contract_addr, &amount_b);

        // Update pool state.
        pool.reserve_a += amount_a;
        pool.reserve_b += amount_b;
        pool.lp_total_supply += lp_minted;
        set_pool(&env, &pool);

        // Credit LP tokens to provider.
        let current_lp = get_lp_balance(&env, pool_id, &provider);
        set_lp_balance(&env, pool_id, &provider, current_lp + lp_minted);

        // CRITICAL: Invalidate query cache after state change (Issue #215)
        storage::invalidate_query_cache(&env, pool_id);

        env.events().publish(
            (Symbol::new(&env, "LiquidityAdded"),),
            (pool_id, &provider, amount_a, amount_b, lp_minted),
        );

        lp_minted
    }

    /// Remove liquidity from a pool by burning LP tokens.
    /// Returns (amount_a, amount_b) withdrawn.
    pub fn remove_liquidity(
        env: Env,
        provider: Address,
        pool_id: u64,
        lp_amount: i128,
    ) -> (i128, i128) {
        provider.require_auth();

        assert!(lp_amount > 0, "LP amount must be positive");

        let current_lp = get_lp_balance(&env, pool_id, &provider);
        assert!(current_lp >= lp_amount, "Insufficient LP balance");

        let mut pool = get_pool(&env, pool_id);
        assert!(pool.lp_total_supply > 0, "Pool has no liquidity");

        // Calculate proportional share of reserves.
        let amount_a = (lp_amount * pool.reserve_a) / pool.lp_total_supply;
        let amount_b = (lp_amount * pool.reserve_b) / pool.lp_total_supply;

        assert!(amount_a > 0 && amount_b > 0, "Withdrawal amounts too small");

        // Update pool state.
        pool.reserve_a -= amount_a;
        pool.reserve_b -= amount_b;
        pool.lp_total_supply -= lp_amount;
        set_pool(&env, &pool);

        // Burn LP tokens from provider.
        set_lp_balance(&env, pool_id, &provider, current_lp - lp_amount);

        // Transfer tokens back to provider.
        let contract_addr = env.current_contract_address();
        let token_a_client = token::Client::new(&env, &pool.token_a);
        let token_b_client = token::Client::new(&env, &pool.token_b);

        token_a_client.transfer(&contract_addr, &provider, &amount_a);
        token_b_client.transfer(&contract_addr, &provider, &amount_b);

        // CRITICAL: Invalidate query cache after state change (Issue #215)
        storage::invalidate_query_cache(&env, pool_id);

        env.events().publish(
            (Symbol::new(&env, "LiquidityRemoved"),),
            (pool_id, &provider, amount_a, amount_b, lp_amount),
        );

        (amount_a, amount_b)
    }

    /// Execute a swap on a pool with slippage protection.
    /// `token_in` must match either `token_a` or `token_b` of the pool.
    /// Returns the amount of tokens received.
    pub fn swap(
        env: Env,
        user: Address,
        pool_id: u64,
        token_in: Address,
        amount_in: i128,
        min_amount_out: i128,
    ) -> i128 {
        user.require_auth();

        assert!(amount_in > 0, "Input amount must be positive");

        let mut pool = get_pool(&env, pool_id);
        assert!(
            pool.reserve_a > 0 && pool.reserve_b > 0,
            "Pool has no liquidity"
        );

        // Determine swap direction.
        let (reserve_in, reserve_out, is_a_to_b) = if token_in == pool.token_a {
            (pool.reserve_a, pool.reserve_b, true)
        } else if token_in == pool.token_b {
            (pool.reserve_b, pool.reserve_a, false)
        } else {
            panic!("Token not in pool");
        };

        // Constant product formula with fee:
        // amount_out = (reserve_out * amount_in_after_fee) / (reserve_in + amount_in_after_fee)
        let fee_factor = 10_000 - pool.fee_bps as i128;
        let amount_in_after_fee = (amount_in * fee_factor) / 10_000;
        let numerator = reserve_out * amount_in_after_fee;
        let denominator = reserve_in + amount_in_after_fee;
        let amount_out = numerator / denominator;

        assert!(amount_out >= min_amount_out, "Slippage tolerance exceeded");
        assert!(amount_out > 0, "Output amount is zero");

        // Transfer input tokens from user to contract.
        let contract_addr = env.current_contract_address();
        let token_in_client = token::Client::new(&env, &token_in);
        token_in_client.transfer(&user, &contract_addr, &amount_in);

        // Transfer output tokens from contract to user.
        let token_out = if is_a_to_b {
            &pool.token_b
        } else {
            &pool.token_a
        };
        let token_out_client = token::Client::new(&env, token_out);
        token_out_client.transfer(&contract_addr, &user, &amount_out);

        // Update reserves (full amount_in goes to reserve, including fee).
        if is_a_to_b {
            pool.reserve_a += amount_in;
            pool.reserve_b -= amount_out;
        } else {
            pool.reserve_b += amount_in;
            pool.reserve_a -= amount_out;
        }
        set_pool(&env, &pool);

        // CRITICAL: Invalidate query cache after state change (Issue #215)
        storage::invalidate_query_cache(&env, pool_id);

        env.events().publish(
            (Symbol::new(&env, "Swapped"),),
            (pool_id, &user, &token_in, amount_in, amount_out),
        );

        amount_out
    }

    /// Get pool information.
    pub fn get_pool(env: Env, pool_id: u64) -> Pool {
        get_pool(&env, pool_id)
    }

    /// Get the current price of a token in the pool (as the ratio of the other reserve).
    /// Returns price scaled by 1_000_000 (6 decimal places) for precision.
    pub fn get_price(env: Env, pool_id: u64, token: Address) -> i128 {
        let pool = get_pool(&env, pool_id);
        assert!(
            pool.reserve_a > 0 && pool.reserve_b > 0,
            "Pool has no liquidity"
        );

        let scale: i128 = 1_000_000;
        if token == pool.token_a {
            // Price of token_a in terms of token_b.
            (pool.reserve_b * scale) / pool.reserve_a
        } else if token == pool.token_b {
            // Price of token_b in terms of token_a.
            (pool.reserve_a * scale) / pool.reserve_b
        } else {
            panic!("Token not in pool");
        }
    }

    /// Get the LP token balance for a provider in a pool.
    pub fn get_lp_balance(env: Env, pool_id: u64, provider: Address) -> i128 {
        get_lp_balance(&env, pool_id, &provider)
    }
}

/// Integer square root using Newton's method (for no_std environments).
fn isqrt(n: i128) -> i128 {
    if n < 0 {
        panic!("Cannot take square root of negative number");
    }
    if n == 0 {
        return 0;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}
