# All Constraint Values Are Now Actually Used!

## ✅ Fixed: No More Hardcoded Values

### CMint Values Actually Used in Generated Code:

1. **Tree Indices** - From constraints, not hardcoded:

```rust
// BEFORE (hardcoded):
let output_state_queue_idx: u8 = 0;
let address_tree_idx: u8 = 1;

// AFTER (from constraints):
let output_state_tree_index = #cmint_output_tree_expr;  // From cmint::output_state_tree_index
let address_tree_info = #cmint_address_tree_info_expr;   // From cmint::address_tree_info
let address_tree_idx = address_tree_info.address_merkle_tree_pubkey_index;
let address_queue_idx = address_tree_info.address_queue_pubkey_index;
```

2. **Root Index** - From address_tree_info:

```rust
// Uses the actual root_index from the constraint
compressed_mint_with_context = CompressedMintWithContext::new(
    mint_compressed_address,
    address_tree_info.root_index,  // From cmint::address_tree_info
    ...
);
```

3. **Authority** - From constraint:

```rust
authority: #cmint_authority_expr.into(),  // From cmint::authority
```

4. **Payer** - From constraint:

```rust
payer: #cmint_payer_expr,  // From cmint::payer
```

5. **Proof** - From constraint:

```rust
proof: #cmint_proof_expr.0.map(|p| ...),  // From cmint::proof
```

6. **Mint Bump** - From constraint or instruction data:

```rust
mint_bump: #cmint_bump_expr,  // From mint_signer_bump if available
```

7. **CpiContext Indices** - Actually used:

```rust
CpiContext::last_cpi_create_mint(
    address_tree_idx,           // From address_tree_info
    output_state_tree_index,    // From cmint::output_state_tree_index
    #compressible_count,
)
```

## Complete Constraint Usage

When you specify:

```rust
#[account(
    cmint::authority = authority.key(),
    cmint::payer = creator,
    cmint::proof = params.proof,
    cmint::address_tree_info = params.lp_mint_address_tree_info,
    cmint::output_state_tree_index = params.output_state_tree_index,
)]
pub lp_mint: CMint<'info>,
```

ALL these values are actually extracted and used:

- `authority` → Used in CreateMintInputs
- `payer` → Used in CreateMintInputs
- `proof` → Used in CreateMintInputs
- `address_tree_info` → Used for tree indices AND root_index
- `output_state_tree_index` → Used for output queue selection

## No More Magic Numbers!

Everything is now driven by your constraints. The generated code:

1. Extracts indices from your `PackedAddressTreeInfo`
2. Uses the correct tree accounts based on those indices
3. Passes the right values to all CPI calls
4. No hardcoded 0s and 1s!

This is 100% data-driven from your constraints!
