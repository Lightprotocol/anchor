# Updating the Caller Program to Use CMint

## Step 1: Update the Account Type

Change `lp_mint` from `UncheckedAccount` to `CMint`:

```rust
// BEFORE:
/// CHECK: checked via mint_signer.
pub lp_mint: UncheckedAccount<'info>,

// AFTER:
#[account(
    cmint::authority = authority,
    cmint::decimals = 9,
    cmint::mint_signer = lp_mint_signer,  // Seeds auto-extracted!
)]
pub lp_mint: CMint<'info>,
```

## Step 2: Add Import

Add the CMint import at the top of the file:

```rust
use anchor_lang::accounts::CMint;
```

## Step 3: Update Instruction Logic

In your instruction handler, use the CMint's methods:

```rust
pub fn initialize<'info>(
    ctx: Context<'_, '_, '_, 'info, Initialize<'info>>,
    init_amount_0: u64,
    init_amount_1: u64,
    mut open_time: u64,
    compression_params: InitializeCompressionParams,
) -> Result<()> {
    // ... existing logic ...

    // Queue mint actions (these will be batched)
    ctx.accounts.lp_mint.add_mint_action(
        MintActionType::MintToCToken {
            account: ctx.accounts.creator_lp_token.key(),
            amount: user_lp_amount,
        }
    )?;

    ctx.accounts.lp_mint.add_mint_action(
        MintActionType::MintToCToken {
            account: ctx.accounts.lp_vault.key(),
            amount: vault_lp_amount,
        }
    )?;

    // ... rest of logic ...

    // Note: The finalize method is called automatically by Anchor!
    // It will batch all mint actions and compressed PDAs into a single CPI
}
```

## Step 4: Remove Manual CPI Code

Remove the manual `create_and_mint_lp` function call since it's now handled automatically:

```rust
// REMOVE THIS:
create_and_mint_lp(
    ctx.accounts.creator.to_account_info(),
    ctx.accounts.authority.to_account_info(),
    &ctx.accounts.lp_mint.key(),
    ctx.accounts.lp_vault.to_account_info(),
    ctx.accounts.creator_lp_token.to_account_info(),
    ctx.accounts.lp_mint_signer.to_account_info(),
    &pool_state_key,
    ctx.accounts.compressed_token_program_cpi_authority.to_account_info(),
    ctx.accounts.compressed_token_program.to_account_info(),
    ctx.bumps.lp_mint_signer,
    &compression_params,
    &cpi_accounts,
    user_lp_amount,
    vault_lp_amount,
    pool_auth_bump,
)?;
```

## Step 5: Ensure Proper Account Ordering

Make sure accounts are in the correct order for the CPI. The CMint finalize will expect:

- compressed_token_program_cpi_authority
- compressed_token_program
- authority (from cmint::authority)
- mint_signer (from cmint::mint_signer)
- creator (fee payer)
- Recipients for mint actions (creator_lp_token, lp_vault)

## Full Example

```rust
#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub creator: Signer<'info>,

    pub amm_config: Box<Account<'info, AmmConfig>>,

    #[account(
        seeds = [AUTH_SEED.as_bytes()],
        bump,
    )]
    pub authority: UncheckedAccount<'info>,

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
        compress_on_init,  // Compress immediately
    )]
    pub pool_state: Box<Account<'info, PoolState>>,

    // ... other accounts ...

    #[account(
        seeds = [
            POOL_LP_MINT_SEED.as_bytes(),
            pool_state.key().as_ref(),
        ],
        bump,
    )]
    pub lp_mint_signer: UncheckedAccount<'info>,

    #[account(
        cmint::authority = authority,
        cmint::decimals = 9,
        cmint::mint_signer = lp_mint_signer,  // Auto-extracts seeds!
    )]
    pub lp_mint: CMint<'info>,

    // ... rest of accounts ...
}
```

## Benefits

1. **No Redundant Seeds**: Seeds are defined once on `lp_mint_signer` and auto-extracted
2. **Type Safety**: `CMint` type ensures proper handling
3. **Automatic Batching**: All mint actions and compressions batched into single CPI
4. **Cleaner Code**: Less manual CPI management
5. **Anchor-Native**: Follows Anchor's patterns and conventions
