use soroban_sdk::{Address, Env};

use crate::types::{DataKey, Pool};

/* ---------------- ADMIN ---------------- */

pub fn set_admin(env: &Env, admin: &Address) {
    env.storage().instance().set(&DataKey::Admin, admin);
}

pub fn get_admin(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&DataKey::Admin)
        .expect("Contract not initialized")
}

pub fn require_admin(env: &Env, caller: &Address) {
    let admin = get_admin(env);
    if caller != &admin {
        panic!("Unauthorized: caller is not admin");
    }
}

/* ---------------- POOL COUNTER ---------------- */

pub fn get_pool_counter(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&DataKey::PoolCounter)
        .unwrap_or(0)
}

pub fn set_pool_counter(env: &Env, counter: u64) {
    env.storage()
        .instance()
        .set(&DataKey::PoolCounter, &counter);
}

/* ---------------- POOL DATA ---------------- */

pub fn set_pool(env: &Env, pool: &Pool) {
    env.storage()
        .persistent()
        .set(&DataKey::Pool(pool.pool_id), pool);
}

pub fn get_pool(env: &Env, pool_id: u64) -> Pool {
    env.storage()
        .persistent()
        .get(&DataKey::Pool(pool_id))
        .expect("Pool not found")
}

/* ---------------- LP BALANCES ---------------- */

pub fn get_lp_balance(env: &Env, pool_id: u64, provider: &Address) -> i128 {
    env.storage()
        .persistent()
        .get(&DataKey::LpBalance(pool_id, provider.clone()))
        .unwrap_or(0)
}

pub fn set_lp_balance(env: &Env, pool_id: u64, provider: &Address, amount: i128) {
    env.storage()
        .persistent()
        .set(&DataKey::LpBalance(pool_id, provider.clone()), &amount);
}

/* ---------------- QUERY CACHE INVALIDATION (Issue #215) ---------------- */

/// Invalidate query cache for a specific pool
/// Called after all state-mutating operations to prevent stale data
pub fn invalidate_query_cache(env: &Env, pool_id: u64) {
    // Remove cached price data
    let price_cache_key = DataKey::PriceCache(pool_id);
    env.storage().instance().remove(&price_cache_key);
    
    // Remove cached reserve data  
    let reserve_cache_key = DataKey::ReserveCache(pool_id);
    env.storage().instance().remove(&reserve_cache_key);
    
    // Emit cache invalidation event for monitoring
    env.events().publish(
        (soroban_sdk::Symbol::new(env, "CacheInvalidated"),),
        (pool_id, env.ledger().timestamp()),
    );
}

/// Invalidate all query caches (for major state changes)
pub fn invalidate_all_query_caches(env: &Env) {
    // Instance-level caches are cleared automatically on TTL expiry
    // This function triggers immediate invalidation
    let invalidation_timestamp = env.ledger().timestamp();
    env.storage()
        .instance()
        .set(&DataKey::LastCacheInvalidation, &invalidation_timestamp);
}

