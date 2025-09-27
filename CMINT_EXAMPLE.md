# CMint with Auto-Extracted Seeds

## Before (Redundant Seeds)

```rust
#[derive(Accounts)]
pub struct Initialize<'info> {
    // ... other accounts ...

    /// Signer PDA used to derive lp_mint and its compressed address
    #[account(
        seeds = [
            POOL_LP_MINT_SEED.as_bytes(),
            pool_state.key().as_ref(),
        ],
        bump,
    )]
    pub lp_mint_signer: UncheckedAccount<'info>,

    /// The compressed mint
    #[account(
        cmint::authority = authority,
        cmint::decimals = 9,
        cmint::mint_signer = lp_mint_signer,
        cmint::mint_signer_seeds = [  // REDUNDANT! Already defined on lp_mint_signer
            POOL_LP_MINT_SEED.as_bytes(),
            pool_state.key().as_ref(),
        ],
        cmint::mint_signer_bump,  // REDUNDANT! Already defined on lp_mint_signer
    )]
    pub lp_mint: CMint<'info>,

    // ... other accounts ...
}
```

## After (Auto-Extracted Seeds)

```rust
#[derive(Accounts)]
pub struct Initialize<'info> {
    // ... other accounts ...

    /// Signer PDA used to derive lp_mint and its compressed address
    #[account(
        seeds = [
            POOL_LP_MINT_SEED.as_bytes(),
            pool_state.key().as_ref(),
        ],
        bump,
    )]
    pub lp_mint_signer: UncheckedAccount<'info>,

    /// The compressed mint - seeds are auto-extracted from lp_mint_signer!
    #[account(
        cmint::authority = authority,
        cmint::decimals = 9,
        cmint::mint_signer = lp_mint_signer,  // Seeds and bump auto-extracted from here!
        // NO NEED for cmint::mint_signer_seeds or cmint::mint_signer_bump
    )]
    pub lp_mint: CMint<'info>,

    // ... other accounts ...
}
```

## How It Works

When you specify `cmint::mint_signer = lp_mint_signer`, Anchor will:

1. Look up the `lp_mint_signer` account in your struct
2. Extract its `seeds` and `bump` constraints
3. Automatically use those for the CMint's mint_signer_seeds and mint_signer_bump
4. Pass the correct AccountInfo and seeds to the compressed token CPI

This follows Anchor's philosophy of reducing redundancy and making the framework more ergonomic.

## Key Relationships

```
mint_signer (PDA) → used as mint_seed → spl_mint (PDA) → compressed_mint
```

- `mint_signer`: The PDA that acts as the seed for deriving the SPL mint
- `spl_mint` (lp_mint): Derived from mint_signer, represents the SPL mint address
- `compressed_mint`: Derived from spl_mint address

## Manual Override Still Possible

If needed, you can still manually specify seeds (they will override the auto-extraction):

```rust
#[account(
    cmint::authority = authority,
    cmint::decimals = 9,
    cmint::mint_signer = lp_mint_signer,
    cmint::mint_signer_seeds = [...],  // Manual override
    cmint::mint_signer_bump = ...,      // Manual override
)]
pub lp_mint: CMint<'info>,
```
