# Anchor Compression Modes for PDAs

## Two Compression Modes

### 1. `compress_on_init` - Immediate Compression with Auto-Close

```rust
#[derive(Accounts)]
pub struct Initialize<'info> {
    /// Pool state - will be compressed immediately and auto-closed
    #[account(
        init,
        compress_on_init,  // ← Compress immediately + auto-close
        seeds = [/* ... */],
        bump,
        payer = creator,
        space = PoolState::INIT_SPACE
    )]
    pub pool_state: Box<Account<'info, PoolState>>,

    /// Observation state - also compressed immediately
    #[account(
        init,
        compress_on_init,  // ← Compress immediately + auto-close
        seeds = [/* ... */],
        bump,
        payer = creator,
        space = ObservationState::INIT_SPACE
    )]
    pub observation_state: Box<Account<'info, ObservationState>>,

    // Required compression accounts (by name convention)
    pub compression_config: AccountInfo<'info>,
    #[account(mut)]
    pub rent_recipient: AccountInfo<'info>,
    // ... other compression accounts
}

pub fn initialize(ctx: Context<Initialize>, params: CompressionParams) -> Result<()> {
    // Your logic here...

    // NO NEED to manually call .close() - Anchor auto-closes at end!
    // pool_state.close() and observation_state.close() are called automatically

    Ok(())
}
```

**What Anchor Does:**

1. Calls `prepare_accounts_for_compression_on_init` for each account
2. Batches ALL accounts into ONE `invoke_light_system_program_cpi_context`
3. **Automatically calls `.close()` on each account at the end**

### 2. `compressible` - Prepare for Future Compression (No Auto-Close)

```rust
#[derive(Accounts)]
pub struct Initialize<'info> {
    /// Pool state - prepared for compression but NOT auto-closed
    #[account(
        init,
        compressible,  // ← Prepare for future compression, no auto-close
        seeds = [/* ... */],
        bump,
        payer = creator,
        space = PoolState::INIT_SPACE
    )]
    pub pool_state: Box<Account<'info, PoolState>>,

    /// Observation state - also prepared but not auto-closed
    #[account(
        init,
        compressible,  // ← Prepare for future compression, no auto-close
        seeds = [/* ... */],
        bump,
        payer = creator,
        space = ObservationState::INIT_SPACE
    )]
    pub observation_state: Box<Account<'info, ObservationState>>,

    // Same compression accounts required
    pub compression_config: AccountInfo<'info>,
    #[account(mut)]
    pub rent_recipient: AccountInfo<'info>,
    // ...
}

pub fn initialize(ctx: Context<Initialize>, params: CompressionParams) -> Result<()> {
    // Your logic here...

    // Accounts are prepared for compression but NOT closed
    // You can compress them later in another instruction

    Ok(())
}
```

**What Anchor Does:**

1. Calls `prepare_accounts_for_empty_compression_on_init` for each account (currently unimplemented)
2. Batches ALL accounts into ONE system CPI
3. **Does NOT call `.close()` - accounts remain active**

## Important Rules

### ✅ Valid: All accounts use the same mode

```rust
// All compress_on_init - OK
#[account(init, compress_on_init)]
pub pool_state: Account<'info, PoolState>,

#[account(init, compress_on_init)]
pub observation_state: Account<'info, ObservationState>,
```

```rust
// All compressible - OK
#[account(init, compressible)]
pub pool_state: Account<'info, PoolState>,

#[account(init, compressible)]
pub observation_state: Account<'info, ObservationState>,
```

### ❌ Invalid: Mixed modes (compile error)

```rust
// ERROR: Cannot mix modes!
#[account(init, compress_on_init)]
pub pool_state: Account<'info, PoolState>,

#[account(init, compressible)]  // ← Different mode = ERROR
pub observation_state: Account<'info, ObservationState>,
```

## Combined with CMint

Both modes work with CMint:

```rust
#[derive(Accounts)]
pub struct Initialize<'info> {
    // CPDAs with compress_on_init (auto-close)
    #[account(init, compress_on_init)]
    pub pool_state: Box<Account<'info, PoolState>>,

    #[account(init, compress_on_init)]
    pub observation_state: Box<Account<'info, ObservationState>>,

    // Compressed mint
    #[account(
        cmint::authority = authority,
        cmint::decimals = 9,
        // ... other cmint constraints
    )]
    pub lp_mint: CMint<'info>,

    // ... other accounts
}
```

**Execution Order:**

1. All CPDAs compressed first (indices 0, 1, ...)
2. CMint created last (index N)
3. Single batched `invoke_signed` for everything
4. If `compress_on_init`: auto-close CPDAs

## Migration Path

### From Manual Compression

```rust
// Before (Manual):
compress_pool_and_observation_pdas(&cpi_accounts, &pool_state, &observation_state, ...)?;
create_and_mint_lp(...)?;
pool_state.close(rent_recipient)?;
observation_state.close(rent_recipient)?;
```

### To Anchor Compression

```rust
// After (Anchor with compress_on_init):
// Just mark accounts and let Anchor handle everything!
#[account(init, compress_on_init)]
pub pool_state: Box<Account<'info, PoolState>>,

// In handler:
ctx.accounts.lp_mint.mint_to(&recipient, amount)?;
// That's it! Compression and close happen automatically
```

## Current Status

- ✅ `compress_on_init`: Fully implemented with auto-close
- ⚠️ `compressible`: Structure in place, but `prepare_accounts_for_empty_compression_on_init` throws `unimplemented!()`
- ✅ Error checking: Cannot mix modes
- ✅ Works with CMint in both modes
- ✅ Proper batching and CPI context chaining
