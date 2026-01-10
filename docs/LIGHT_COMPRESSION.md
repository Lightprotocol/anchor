# Light Protocol Compression Integration

## Fork Changes

This fork adds compressed token support to anchor-spl's `token_interface`:

```rust
// anchor-spl/src/token_interface.rs
pub const COMPRESSED_TOKEN_ID: Pubkey = ...;
static IDS: [Pubkey; 3] = [spl_token::ID, spl_token_2022::ID, COMPRESSED_TOKEN_ID];
```

## TS SDK

The TS SDK adds `decompressIfNeeded()` to method builders:

```typescript
await program.methods
  .swap(amount)
  .decompressIfNeeded()
  .rpc();
```

## Rust Macros (light-sdk-macros)

### `#[add_compressible_instructions]`

Generates compress/decompress instructions for listed accounts.

```rust
#[add_compressible_instructions(
    PoolState = (POOL_SEED, ctx.accounts.amm_config, ctx.accounts.token_0_mint, ctx.accounts.token_1_mint),
)]
#[program]
pub mod my_program { ... }
```

### `#[derive(LightFinalize)]` + `#[compressible]`

Auto-compresses PDAs at instruction end.

```rust
#[derive(Accounts, LightFinalize)]
#[instruction(params: MyParams)]
pub struct MyInstruction<'info> {
    #[account(mut)]
    pub creator: Signer<'info>,

    #[account(init, payer = creator, space = 8 + MyAccount::INIT_SPACE, seeds = [...], bump)]
    #[compressible(
        address_tree_info = params.address_tree_info,
        output_tree = params.output_state_tree_index
    )]
    pub my_account: Account<'info, MyAccount>,

    pub compression_config: AccountInfo<'info>,
}
```

### `#[light_instruction(params)]`

Auto-calls `light_finalize()` at instruction end.

```rust
#[light_instruction(params)]
pub fn create_account(ctx: Context<MyInstruction>, params: MyParams) -> Result<()> {
    // your logic
    Ok(())  // light_finalize auto-called here
}
```

### `#[light_mint]` (single mint)

Creates a compressed mint at instruction end.

```rust
#[derive(Accounts, LightFinalize)]
#[instruction(params: MyParams)]
pub struct CreateMint<'info> {
    #[account(mut)]
    pub creator: Signer<'info>,

    /// CHECK: Mint signer PDA that seeds the mint address
    pub mint_signer: UncheckedAccount<'info>,

    /// CHECK: The mint account to be created
    #[account(mut)]
    #[light_mint(
        mint_signer = mint_signer,
        authority = authority,
        decimals = 9,
        address_tree_info = params.mint_address_tree_info,
        output_tree = params.output_state_tree_index
    )]
    pub my_mint: UncheckedAccount<'info>,

    pub authority: UncheckedAccount<'info>,
    pub compression_config: AccountInfo<'info>,
}
```

## Required Program Setup

```rust
use light_sdk_types::CpiSigner;
use light_macros::derive_light_cpi_signer;

pub const LIGHT_CPI_SIGNER: CpiSigner = derive_light_cpi_signer!("YOUR_PROGRAM_ID");
```

## Params Struct Requirements

Your params struct (first arg of `#[instruction(...)]`) must have:

```rust
pub struct MyParams {
    pub address_tree_info: PackedAddressTreeInfo,  // for each compressible/mint
    pub output_state_tree_index: u8,
    pub proof: ValidityProof,  // required for mint creation
}
```

## Current Limitations

- **Multiple mints**: Not supported via macro. Use `light_ctoken_sdk::ctoken::CreateCMintCpi` directly with CPI context batching.
- **Mixed PDAs + mints**: Not supported via macro. Handle mints manually.
- **Token accounts**: Use `light_ctoken_sdk` directly (`CreateCTokenAccountCpi`, `CTokenMintToCpi`).

## Gotchas

1. **Fee payer field**: Must be named `fee_payer`, `payer`, or `creator`
2. **compression_config field**: Required as `AccountInfo<'info>`
3. **remaining_accounts**: Light system accounts come via remaining_accounts - client must pass them in v2 format
4. **PackedAddressTreeInfo fields**: Uses `address_merkle_tree_pubkey_index`, `address_queue_pubkey_index`, `root_index`

## raydium-cp-swap Usage

Currently uses:
- `#[compressible]` on `pool_state` for auto-compression
- Manual `CreateCTokenAccountCpi` for token vaults
- Manual mint handling (LP mint created by client, no `#[light_mint]`)
