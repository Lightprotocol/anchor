# Complete Anchor Compressed Implementation Example

## Updated Caller Program with Compressed Support

```rust
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct Initialize<'info> {
    /// Address paying to create the pool
    #[account(mut)]
    pub creator: Signer<'info>,

    /// AMM config
    pub amm_config: Box<Account<'info, AmmConfig>>,

    /// Pool authority
    #[account(
        seeds = [crate::AUTH_SEED.as_bytes()],
        bump,
    )]
    pub authority: UncheckedAccount<'info>,

    /// Pool state - COMPRESSIBLE (index 0)
    #[account(
        init,
        compressible,  // ← Makes this account compressible
        seeds = [
            POOL_SEED.as_bytes(),
            amm_config.key().as_ref(),
            token_0_mint.key().as_ref(),
            token_1_mint.key().as_ref(),
        ],
        bump,
        payer = creator,
        space = PoolState::INIT_SPACE
    )]
    pub pool_state: Box<Account<'info, PoolState>>,

    /// Token mints...
    #[account(mint::token_program = token_0_program)]
    pub token_0_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(mint::token_program = token_1_program)]
    pub token_1_mint: Box<InterfaceAccount<'info, Mint>>,

    /// LP mint signer PDA
    #[account(
        seeds = [POOL_LP_MINT_SEED.as_bytes(), pool_state.key().as_ref()],
        bump,
    )]
    pub lp_mint_signer: UncheckedAccount<'info>,

    /// Compressed LP mint (index 2, after CPDAs)
    #[account(
        cmint::authority = authority,
        cmint::decimals = 9,
        cmint::mint_signer_seeds = [POOL_LP_MINT_SEED.as_bytes(), pool_state.key().as_ref()],
        cmint::mint_signer_bump,
        cmint::program_authority_seeds = [crate::AUTH_SEED.as_bytes()],
        cmint::program_authority_bump
    )]
    pub lp_mint: CMint<'info>,

    // Token accounts...
    #[account(mut)]
    pub creator_token_0: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(mut)]
    pub creator_token_1: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(mut)]
    pub creator_lp_token: UncheckedAccount<'info>,

    #[account(mut, seeds = [POOL_VAULT_SEED.as_bytes(), lp_mint.key().as_ref()], bump)]
    pub lp_vault: UncheckedAccount<'info>,

    // Token vaults...
    #[account(mut, seeds = [POOL_VAULT_SEED.as_bytes(), pool_state.key().as_ref(), token_0_mint.key().as_ref()], bump)]
    pub token_0_vault: UncheckedAccount<'info>,

    #[account(mut, seeds = [POOL_VAULT_SEED.as_bytes(), pool_state.key().as_ref(), token_1_mint.key().as_ref()], bump)]
    pub token_1_vault: UncheckedAccount<'info>,

    /// Observation state - COMPRESSIBLE (index 1)
    #[account(
        init,
        compressible,  // ← Makes this account compressible
        seeds = [
            OBSERVATION_SEED.as_bytes(),
            pool_state.key().as_ref(),
        ],
        bump,
        payer = creator,
        space = ObservationState::INIT_SPACE
    )]
    pub observation_state: Box<Account<'info, ObservationState>>,

    // Standard programs...
    pub token_program: Program<'info, Token>,
    pub token_0_program: Interface<'info, TokenInterface>,
    pub token_1_program: Interface<'info, TokenInterface>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,

    // Compression-specific accounts (required by name convention)
    /// CHECK: Compression config
    pub compression_config: AccountInfo<'info>,

    /// CHECK: Rent recipient
    #[account(mut)]
    pub rent_recipient: AccountInfo<'info>,

    /// CHECK: Compressed token program CPI authority
    pub compressed_token_program_cpi_authority: AccountInfo<'info>,

    /// CHECK: Compressed token program
    pub compressed_token_program: AccountInfo<'info>,

    // ... other compression accounts ...
}

pub fn initialize<'info>(
    ctx: Context<'_, '_, '_, 'info, Initialize<'info>>,
    init_amount_0: u64,
    init_amount_1: u64,
    open_time: u64,
    compression_params: InitializeCompressionParams,
) -> Result<()> {
    // Your normal initialization logic
    let pool_state = &mut ctx.accounts.pool_state;
    let observation_state = &mut ctx.accounts.observation_state;

    pool_state.initialize(/* ... */);
    observation_state.pool_id = pool_state.key();

    // Transfer initial liquidity
    transfer_from_user_to_pool_vault(/* ... */)?;

    // Queue mint actions for compressed mint
    let user_lp_amount = calculate_user_lp(/* ... */);
    let vault_lp_amount = calculate_vault_lp(/* ... */);

    ctx.accounts.lp_mint.mint_to(&ctx.accounts.creator_lp_token.key(), user_lp_amount)?;
    ctx.accounts.lp_mint.mint_to(&ctx.accounts.lp_vault.key(), vault_lp_amount)?;

    // Close accounts to trigger compression
    pool_state.close(ctx.accounts.rent_recipient.clone())?;
    observation_state.close(ctx.accounts.rent_recipient.clone())?;

    Ok(())
}

#[derive(AnchorSerialize, AnchorDeserialize)]
pub struct InitializeCompressionParams {
    pub pool_address_tree_info: PackedAddressTreeInfo,
    pub observation_address_tree_info: PackedAddressTreeInfo,
    pub lp_mint_address_tree_info: PackedAddressTreeInfo,
    pub lp_mint_bump: u8,
    pub creator_lp_token_bump: u8,
    pub proof: ValidityProof,
    pub output_state_tree_index: u8,
}
```

## What Anchor Does Behind the Scenes

### 1. Scans for Compressible Accounts

- Detects `pool_state` with `init, compressible` → index 0
- Detects `observation_state` with `init, compressible` → index 1
- Detects `lp_mint: CMint` → index 2 (always last)

### 2. Generates Batched Compression Logic

```rust
// Auto-generated in AccountsFinalize implementation
{
    // Build CPI accounts
    let cpi_accounts = CpiAccountsSmall::new_with_config(
        &self.creator,
        _remaining,
        CpiAccountsConfig::new_with_cpi_context(crate::LIGHT_CPI_SIGNER),
    );

    // Load compression config
    let compression_config = CompressibleConfig::load_checked(&self.compression_config, &crate::ID)?;

    // Parse compression params from ix_data
    let compression_params = /* deserialize from ix_data */;

    // Collect ALL compressible accounts
    let mut all_compressed_infos = Vec::new();

    // Process pool_state (index 0)
    let pool_new_address_params = compression_params.pool_address_tree_info
        .into_new_address_params_assigned_packed(
            self.pool_state.key().to_bytes(),
            true,
            Some(0),
        );

    let pool_compressed_address = derive_address(/* ... */);

    let pool_compressed_infos = prepare_accounts_for_compression_on_init::<PoolState>(
        &[&*self.pool_state],
        &[pool_compressed_address],
        &[pool_new_address_params],
        &[compression_params.output_state_tree_index],
        &cpi_accounts,
        &address_space,
        &self.rent_recipient,
    )?;
    all_compressed_infos.extend(pool_compressed_infos);

    // Process observation_state (index 1)
    let observation_new_address_params = compression_params.observation_address_tree_info
        .into_new_address_params_assigned_packed(
            self.observation_state.key().to_bytes(),
            true,
            Some(1),
        );

    let observation_compressed_address = derive_address(/* ... */);

    let observation_compressed_infos = prepare_accounts_for_compression_on_init::<ObservationState>(
        &[&*self.observation_state],
        &[observation_compressed_address],
        &[observation_new_address_params],
        &[compression_params.output_state_tree_index],
        &cpi_accounts,
        &address_space,
        &self.rent_recipient,
    )?;
    all_compressed_infos.extend(observation_compressed_infos);

    // ONE batched system CPI for ALL CPDAs
    let cpi_inputs = CpiInputs::new_first_cpi(
        all_compressed_infos,
        vec![pool_new_address_params, observation_new_address_params],
    );

    let cpi_context = cpi_accounts.cpi_context().unwrap();
    let cpi_context_accounts = CpiContextWriteAccounts {
        fee_payer: cpi_accounts.fee_payer(),
        authority: cpi_accounts.authority().unwrap(),
        cpi_context,
        cpi_signer: crate::LIGHT_CPI_SIGNER,
    };
    cpi_inputs.invoke_light_system_program_cpi_context(cpi_context_accounts)?;

    // Then chain CMint creation as last CPI
    let mint_actions = self.lp_mint.take_actions();
    if !mint_actions.is_empty() {
        // Create mint and execute mint_to actions
        // Using last_cpi_create_mint with index 2
        // ... (mint creation logic)
    }
}
```

## Key Benefits

1. **Clean Syntax**: Just add `compressible` to any `Account` with `init`
2. **Automatic Batching**: All CPDAs collected and compressed in ONE system CPI
3. **Proper Ordering**: CPDAs first (indices 0, 1, ...), CMint always last
4. **Context Chaining**: CPI context properly carried through to CMint
5. **Generic**: Works for 1-N compressible accounts
6. **Orthodox**: Uses standard Anchor patterns, no new types needed

## Migration from Manual

Before (Manual):

```rust
compress_pool_and_observation_pdas(&cpi_accounts, &pool_state, &observation_state, ...)?;
create_and_mint_lp(...)?;
pool_state.close(rent_recipient)?;
observation_state.close(rent_recipient)?;
```

After (Anchor):

```rust
// Just mark accounts as compressible and queue mint actions
// Anchor handles ALL compression logic automatically!
ctx.accounts.lp_mint.mint_to(&recipient, amount)?;
pool_state.close(rent_recipient)?;
observation_state.close(rent_recipient)?;
```
