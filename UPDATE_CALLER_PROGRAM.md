# How to Update Your Caller Program

## The New Constraint System

We now use explicit instruction data mapping with `cpda::` and `cmint::` constraints, following Anchor's `#[instruction(...)]` pattern.

## Step 1: Update Your Accounts Struct

```rust
#[derive(Accounts)]
#[instruction(params: InitializeCompressionParams)]  // Add this!
pub struct Initialize<'info> {
    #[account(mut)]
    pub creator: Signer<'info>,

    // ... other accounts ...

    // For compressed PDAs, use cpda:: constraints
    #[account(
        init,
        seeds = [
            POOL_SEED.as_bytes(),
            amm_config.key().as_ref(),
            token_0_mint.key().as_ref(),
            token_1_mint.key().as_ref(),
        ],
        bump,
        payer = creator,
        space = PoolState::INIT_SPACE,
        cpda::address_tree_info = params.pool_address_tree_info,
        cpda::proof = params.proof,
    )]
    pub pool_state: Box<Account<'info, PoolState>>,

    #[account(
        init,
        seeds = [
            OBSERVATION_SEED.as_bytes(),
            pool_state.key().as_ref(),
        ],
        bump,
        payer = creator,
        space = ObservationState::INIT_SPACE,
        cpda::address_tree_info = params.observation_address_tree_info,
        cpda::proof = params.proof,
    )]
    pub observation_state: Box<Account<'info, ObservationState>>,

    // For compressed mints, use cmint:: constraints
    #[account(
        cmint::authority = authority.key(),
        cmint::decimals = 9,
        cmint::payer = creator,
        cmint::mint_signer = lp_mint_signer,
        cmint::address_tree_info = params.lp_mint_address_tree_info,
        cmint::proof = params.proof,
    )]
    pub lp_mint: CMint<'info>,

    // ... rest of accounts ...
}
```

## Step 2: Update Your Instruction Handler

```rust
pub fn initialize<'info>(
    ctx: Context<'_, '_, '_, 'info, Initialize<'info>>,
    params: InitializeCompressionParams,  // Now just one param struct
) -> Result<()> {
    // ... your logic ...

    // Queue mint actions (batched automatically)
    ctx.accounts.lp_mint.mint_to(
        &ctx.accounts.creator_lp_token.key(),
        user_lp_amount,
    )?;

    ctx.accounts.lp_mint.mint_to(
        &ctx.accounts.lp_vault.key(),
        vault_lp_amount,
    )?;

    // Remove all manual compression code - it's automatic now!
    // No need for:
    // - compress_pool_and_observation_pdas()
    // - create_and_mint_lp()
    // - pool_state.close()
    // - observation_state.close()

    Ok(())
}
```

## Step 3: Keep Your InitializeCompressionParams

Your existing struct is perfect:

```rust
#[derive(AnchorSerialize, AnchorDeserialize, Debug)]
pub struct InitializeCompressionParams {
    // Standard params
    pub init_amount_0: u64,
    pub init_amount_1: u64,
    pub open_time: u64,

    // CPDA params
    pub pool_address_tree_info: PackedAddressTreeInfo,
    pub observation_address_tree_info: PackedAddressTreeInfo,

    // CMint params
    pub lp_mint_address_tree_info: PackedAddressTreeInfo,
    pub lp_mint_bump: u8,
    pub creator_lp_token_bump: u8,

    // Shared
    pub proof: ValidityProof,
    pub output_state_tree_index: u8,
}
```

## What's Automatic Now

1. **CPDA Compression**: Accounts with `cpda::` constraints are automatically compressed
2. **CMint Creation**: The compressed mint is created with all queued actions
3. **Batched CPI**: Everything happens in one `invoke_signed` at the end
4. **Auto-close**: CPDA accounts are automatically closed after compression

## Required Accounts

Make sure you still have these accounts (detected by name):

- `compression_config`
- `compressed_token_program`
- `compressed_token_program_cpi_authority`
- `rent_recipient`

## Benefits

- **Cleaner Code**: Remove all manual compression logic
- **Type Safety**: Anchor validates everything at compile time
- **Orthodox Anchor**: Follows established patterns perfectly
- **Explicit Intent**: Clear data flow from instruction params to constraints
