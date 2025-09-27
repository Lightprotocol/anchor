# Using CPDA and CMint Constraints with #[instruction(...)]

## The Anchor Way: Explicit Instruction Data

Instead of auto-generating compression code that guesses field names, we now require explicit instruction data mapping using Anchor's `#[instruction(...)]` pattern.

## Example Usage

```rust
#[derive(Accounts)]
#[instruction(params: InitializeParams)]  // Access instruction data
pub struct Initialize<'info> {
    #[account(mut)]
    pub creator: Signer<'info>,

    // Compressed PDA with explicit data mapping
    #[account(
        init,
        seeds = [...],
        bump,
        payer = creator,
        space = PoolState::INIT_SPACE,
        cpda::address_tree_info = params.pool_address_tree_info,
        cpda::proof = params.proof,
    )]
    pub pool_state: Box<Account<'info, PoolState>>,

    // Another compressed PDA
    #[account(
        init,
        seeds = [...],
        bump,
        payer = creator,
        space = ObservationState::INIT_SPACE,
        cpda::address_tree_info = params.observation_address_tree_info,
        cpda::proof = params.proof,
    )]
    pub observation_state: Box<Account<'info, ObservationState>>,

    // Compressed Mint with explicit data
    #[account(
        cmint::authority = authority,
        cmint::decimals = 9,
        cmint::payer = creator,
        cmint::mint_signer = lp_mint_signer,
        cmint::address_tree_info = params.lp_mint_address_tree_info,
        cmint::proof = params.proof,
    )]
    pub lp_mint: CMint<'info>,

    // ... other accounts ...
}

#[derive(AnchorSerialize, AnchorDeserialize)]
pub struct InitializeParams {
    // Standard params
    pub init_amount_0: u64,
    pub init_amount_1: u64,
    pub open_time: u64,

    // Compression params for CPDAs
    pub pool_address_tree_info: PackedAddressTreeInfo,
    pub observation_address_tree_info: PackedAddressTreeInfo,

    // Compression params for CMint
    pub lp_mint_address_tree_info: PackedAddressTreeInfo,
    pub lp_mint_bump: u8,
    pub creator_lp_token_bump: u8,

    // Shared proof
    pub proof: ValidityProof,
    pub output_state_tree_index: u8,
}

pub fn initialize(
    ctx: Context<Initialize>,
    params: InitializeParams,
) -> Result<()> {
    // Your logic here

    // Queue mint actions for CMint
    ctx.accounts.lp_mint.mint_to(&recipient1, amount1)?;
    ctx.accounts.lp_mint.mint_to(&recipient2, amount2)?;

    // Everything is batched and executed automatically at the end!
    Ok(())
}
```

## Key Benefits

1. **Explicit Data Mapping**: No guessing field names - you explicitly map instruction data to constraints
2. **Type Safety**: Anchor validates that the instruction data matches what's expected
3. **Orthodox Anchor**: This follows Anchor's established patterns perfectly
4. **Clear Intent**: Anyone reading the code knows exactly where compression params come from

## What Happens Automatically

When you use `cpda::` and `cmint::` constraints:

1. Anchor generates the batched CPI code
2. All compressed PDAs are initialized and compressed in one batch
3. CMint is created with all queued mint actions
4. Everything happens in a single `invoke_signed` at the end

## Required Accounts

For this to work, you still need these accounts in your struct:

- `compression_config`: The compression configuration account
- `compressed_token_program`: The compressed token program
- `compressed_token_program_cpi_authority`: The CPI authority
- And other Light Protocol required accounts

But now they're detected by name automatically!
