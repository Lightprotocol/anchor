# Mint Signer AccountInfo Implementation

## The Issue

You correctly identified that we were collecting the mint_signer seeds for signing but NOT adding the mint_signer AccountInfo to the CPI call. This would cause the CPI to fail because the Light Protocol expects the mint_signer account to be present.

## The Fix

### 1. In `exit.rs` (Code Generation)

✅ Already correctly implemented:

```rust
// Add mint_signer if provided
if let Some(mint_signer) = &self.#cmint_ident.mint_signer {
    account_infos.push(mint_signer.clone());
} else {
    // If no explicit mint_signer provided, use the CMint account itself
    account_infos.push(self.#cmint_ident.to_account_info());
}
```

### 2. In `compressed_mint.rs` (Runtime Implementation)

✅ Now fixed:

```rust
// Build account infos for the CPI
let mut account_infos = vec![
    compressed_token_program.clone(),
    compressed_token_program_cpi_authority.clone(),
    authority.clone(),
];

// Add mint_signer if we have a CMint
if let Some(mint) = mint {
    if let Some(mint_signer) = &mint.mint_signer {
        account_infos.push(mint_signer.clone());
    } else {
        // If no explicit mint_signer provided, use the CMint account itself
        account_infos.push(mint.to_account_info());
    }
}

account_infos.extend([
    payer.clone(),
    system_program.clone(),
]);
```

## Complete Flow

1. **Account Definition**: User defines mint_signer with seeds

   ```rust
   #[account(seeds = [...], bump)]
   pub lp_mint_signer: UncheckedAccount<'info>,

   #[account(cmint::mint_signer = lp_mint_signer)]
   pub lp_mint: CMint<'info>,
   ```

2. **Seeds Auto-Extraction**: The macro extracts seeds from `lp_mint_signer`

3. **AccountInfo Storage**: The mint_signer AccountInfo is stored in CMint struct

   ```rust
   pub struct CMint<'info> {
       pub mint_signer: Option<AccountInfo<'info>>,
       pub mint_signer_seeds: Option<Vec<Vec<u8>>>,
       pub mint_signer_bump: Option<u8>,
       // ...
   }
   ```

4. **CPI Invocation**: Both AccountInfo AND seeds are used
   - AccountInfo added to `account_infos` array for the CPI
   - Seeds used in `signer_seeds` for `invoke_signed`

## Why Both Are Needed

- **AccountInfo**: Required by the CPI instruction to identify the account
- **Seeds**: Required for signing the transaction with `invoke_signed`

The Light Protocol's `create_mint_action_cpi` expects:

1. The mint_signer account to be present in the accounts array
2. The seeds to sign the transaction (proving authority over the PDA)

## Account Order for CPI

The expected account order is:

1. compressed_token_program
2. compressed_token_program_cpi_authority
3. authority
4. **mint_signer** (if CMint present)
5. payer
6. system_program
7. CPDA accounts (if any)
8. Recipient accounts for mint actions
