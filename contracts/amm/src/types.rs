use soroban_sdk::{contracttype, Address};

/// Storage keys for the AMM contract.
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Admin address.
    Admin,
    /// Pool counter for generating unique pool IDs.
    PoolCounter,
    /// Pool data keyed by pool ID.
    Pool(u64),
    /// LP token balance: (pool_id, provider).
    LpBalance(u64, Address),
    /// Cached price data for pool (Issue #215)
    PriceCache(u64),
    /// Cached reserve data for pool (Issue #215)
    ReserveCache(u64),
    /// Timestamp of last cache invalidation (Issue #215)
    LastCacheInvalidation,
}

/// Represents a liquidity pool for a token pair.
#[contracttype]
#[derive(Clone, Debug)]
pub struct Pool {
    /// Unique pool identifier.
    pub pool_id: u64,
    /// Address of token A.
    pub token_a: Address,
    /// Address of token B.
    pub token_b: Address,
    /// Reserve of token A in the pool.
    pub reserve_a: i128,
    /// Reserve of token B in the pool.
    pub reserve_b: i128,
    /// Total supply of LP tokens for this pool.
    pub lp_total_supply: i128,
    /// Fee in basis points (e.g. 30 = 0.30%).
    pub fee_bps: u32,
}
