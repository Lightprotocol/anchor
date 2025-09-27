# Complete CMint and CPDA Constraint System

## ✅ All Constraints Now Available

### CMint Constraints

```rust
#[account(
    cmint::authority = authority.key(),           // Authority for the mint
    cmint::decimals = 9,                          // Mint decimals
    cmint::payer = creator,                       // Who pays for creation
    cmint::mint_signer = lp_mint_signer,          // Linked signer PDA
    cmint::address_tree_info = params.lp_mint_address_tree_info,  // Tree info from ix data
    cmint::proof = params.proof,                  // Proof from ix data
    cmint::output_state_tree_index = params.output_state_tree_index,  // Output tree index
)]
pub lp_mint: CMint<'info>,
```

### CPDA Constraints

```rust
#[account(
    init,
    seeds = [...],
    bump,
    payer = creator,
    space = PoolState::INIT_SPACE,
    compress_on_init,  // Optional: triggers immediate compression + auto-close
    cpda::address_tree_info = params.pool_address_tree_info,
    cpda::proof = params.proof,
    cpda::output_state_tree_index = params.output_state_tree_index,
)]
pub pool_state: Box<Account<'info, PoolState>>,
```

## Complete Example

```rust
#[derive(Accounts)]
#[instruction(params: InitializeCompressionParams)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub creator: Signer<'info>,

    #[account(
        seeds = [b"authority"],
        bump,
    )]
    pub authority: UncheckedAccount<'info>,

    // Compressed PDA with immediate compression
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
        compress_on_init,
        cpda::address_tree_info = params.pool_address_tree_info,
        cpda::proof = params.proof,
        cpda::output_state_tree_index = params.output_state_tree_index,
    )]
    pub pool_state: Box<Account<'info, PoolState>>,

    // Compressed Mint with all params
    #[account(
        cmint::authority = authority.key(),
        cmint::decimals = 9,
        cmint::payer = creator,
        cmint::mint_signer = lp_mint_signer,
        cmint::address_tree_info = params.lp_mint_address_tree_info,
        cmint::proof = params.proof,
        cmint::output_state_tree_index = params.output_state_tree_index,
    )]
    pub lp_mint: CMint<'info>,

    #[account(
        seeds = [POOL_LP_MINT_SEED.as_bytes(), pool_state.key().as_ref()],
        bump,
    )]
    pub lp_mint_signer: UncheckedAccount<'info>,

    // Required supporting accounts (detected by name)
    pub compression_config: AccountInfo<'info>,
    pub rent_recipient: AccountInfo<'info>,
    pub compressed_token_program: AccountInfo<'info>,
    pub compressed_token_program_cpi_authority: AccountInfo<'info>,

    // ... other accounts ...
}

#[derive(AnchorSerialize, AnchorDeserialize)]
pub struct InitializeCompressionParams {
    // Your params
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

    // Shared params
    pub proof: ValidityProof,
    pub output_state_tree_index: u8,
}

pub fn initialize(
    ctx: Context<Initialize>,
    params: InitializeCompressionParams,
) -> Result<()> {
    // Your logic...

    // Queue mint actions (batched automatically)
    ctx.accounts.lp_mint.mint_to(
        &ctx.accounts.creator_lp_token.key(),
        user_lp_amount,
    )?;

    ctx.accounts.lp_mint.mint_to(
        &ctx.accounts.lp_vault.key(),
        vault_lp_amount,
    )?;

    // Everything else is automatic!
    Ok(())
}
```

## What's Generated Automatically

The `CreateMintInputs` struct is populated from your constraints:

```rust
CreateMintInputs {
    compressed_mint_inputs: ...,
    mint_seed: self.lp_mint.key(),
    mint_bump: ...,
    authority: /* from cmint::authority */,
    payer: /* from cmint::payer */,
    proof: /* from cmint::proof */,
    address_tree: /* derived from trees */,
    output_queue: /* derived from output_state_tree_index */,
    actions: /* from your mint_to calls */,
}
```

## Key Points

✅ **All params mapped from constraints** - No hardcoding  
✅ **Explicit data flow** - Clear where each value comes from  
✅ **Type safe** - Compile-time validation  
✅ **Orthodox Anchor** - Follows established patterns  
✅ **Automatic batching** - Everything in one CPI
