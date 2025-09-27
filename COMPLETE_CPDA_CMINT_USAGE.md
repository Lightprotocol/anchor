# Complete CPDA and CMint Constraint System

## The Proper Anchor Way

We now have a complete constraint system that follows Anchor's `#[instruction(...)]` pattern perfectly.

## CPDA (Compressed PDA) Constraints

For accounts that should be compressed:

```rust
#[derive(Accounts)]
#[instruction(params: InitializeCompressionParams)]
pub struct Initialize<'info> {
    // For immediate compression with auto-close
    #[account(
        init,
        seeds = [...],
        bump,
        payer = creator,
        space = PoolState::INIT_SPACE,
        compress_on_init,  // This flag triggers immediate compression
        cpda::address_tree_info = params.pool_address_tree_info,
        cpda::proof = params.proof,
        cpda::output_state_tree_index = params.output_state_tree_index,
    )]
    pub pool_state: Box<Account<'info, PoolState>>,

    // Another compressed PDA
    #[account(
        init,
        seeds = [...],
        bump,
        payer = creator,
        space = ObservationState::INIT_SPACE,
        compress_on_init,  // Immediate compression
        cpda::address_tree_info = params.observation_address_tree_info,
        cpda::proof = params.proof,
        cpda::output_state_tree_index = params.output_state_tree_index,
    )]
    pub observation_state: Box<Account<'info, ObservationState>>,

    // For compressible (prepare only, no auto-close)
    #[account(
        init,
        seeds = [...],
        bump,
        payer = creator,
        space = SomeState::INIT_SPACE,
        cpda::address_tree_info = params.some_address_tree_info,
        cpda::proof = params.proof,
        cpda::output_state_tree_index = params.output_state_tree_index,
        // NO compress_on_init - this will only prepare for future compression
    )]
    pub some_state: Box<Account<'info, SomeState>>,
}
```

## CMint (Compressed Mint) Constraints

```rust
    #[account(
        cmint::authority = authority.key(),
        cmint::decimals = 9,
        cmint::payer = creator,
        cmint::mint_signer = lp_mint_signer,
        cmint::address_tree_info = params.lp_mint_address_tree_info,
        cmint::proof = params.proof,
    )]
    pub lp_mint: CMint<'info>,

    #[account(
        seeds = [POOL_LP_MINT_SEED.as_bytes(), pool_state.key().as_ref()],
        bump,
    )]
    pub lp_mint_signer: UncheckedAccount<'info>,
```

## Instruction Data Structure

```rust
#[derive(AnchorSerialize, AnchorDeserialize)]
pub struct InitializeCompressionParams {
    // Standard params
    pub init_amount_0: u64,
    pub init_amount_1: u64,
    pub open_time: u64,

    // CPDA params - one for each compressed account
    pub pool_address_tree_info: PackedAddressTreeInfo,
    pub observation_address_tree_info: PackedAddressTreeInfo,

    // CMint params
    pub lp_mint_address_tree_info: PackedAddressTreeInfo,
    pub lp_mint_bump: u8,
    pub creator_lp_token_bump: u8,

    // Shared compression params
    pub proof: ValidityProof,
    pub output_state_tree_index: u8,
}
```

## Key Features

### 1. **`compress_on_init` Flag**

- When present: Account is compressed immediately and auto-closed
- When absent: Account is only prepared for future compression

### 2. **Explicit Data Mapping**

- `cpda::address_tree_info` - Maps to instruction data field
- `cpda::proof` - Maps to instruction data field
- `cpda::output_state_tree_index` - Maps to instruction data field

### 3. **CMint Integration**

- All CMint constraints work as before
- `cmint::mint_signer` links to the signer PDA
- Seeds are auto-extracted from the linked account

### 4. **Automatic Batching**

- All compression happens in one `invoke_signed`
- CMint creation and minting batched together
- Auto-close for `compress_on_init` accounts

## What Happens Automatically

1. **For `compress_on_init` accounts:**

   - Compressed immediately during instruction
   - Auto-closed after compression
   - Rent reclaimed to `rent_recipient`

2. **For regular CPDA accounts (no `compress_on_init`):**

   - Prepared for compression
   - Can be compressed later when inactive
   - No auto-close

3. **For CMint:**
   - Created with all mint actions
   - Batched with CPDA compression
   - Single CPI for everything

## Required Supporting Accounts

These must be present (detected by name):

- `compression_config`
- `rent_recipient`
- `compressed_token_program`
- `compressed_token_program_cpi_authority`

## Benefits

✅ **100% Orthodox Anchor** - Follows established patterns  
✅ **Explicit Intent** - Clear data flow via constraints  
✅ **Type Safety** - Compile-time validation  
✅ **Flexible** - Mix compress_on_init and compressible  
✅ **Clean** - No manual compression code needed
