#![allow(clippy::needless_pass_by_value)]
use crate::accounts::cmint::{CMint, MintAction};
use crate::solana_program::account_info::AccountInfo;
use crate::solana_program::pubkey::Pubkey;
use crate::Result;

#[cfg(not(feature = "compressed-mint-light"))]
/// Finalizes a compressed mint by executing any queued actions in a single batch.
///
/// Note: This is a placeholder implementation. Integrations should provide a
/// proper implementation that performs a single invoke_signed for create-mint
/// (if required) and all queued mint_to actions using the Light SDK.
pub fn finalize_cmint<'info>(
    _mint: &CMint<'info>,
    _actions: Vec<MintAction>,
    _remaining: &[AccountInfo<'info>],
    _ix_data: &[u8],
) -> Result<()> {
    // Default no-op to keep core crate free of Light SDK dependency.
    Ok(())
}

#[cfg(not(feature = "compressed-mint-light"))]
/// Context for batched compressed operations (CPDAs and CMints) - stub version
pub struct CompressedBatchContext {
    pub cpda_indices: Vec<u32>,
    pub cmint_index: Option<u32>,
}

#[cfg(not(feature = "compressed-mint-light"))]
/// Stub for finalize_compressed_batch when feature is not enabled
pub fn finalize_compressed_batch<'info>(
    _batch_context: &CompressedBatchContext,
    _mint: Option<&CMint<'info>>,
    _actions: Vec<MintAction>,
    _remaining: &[AccountInfo<'info>],
    _ix_data: &[u8],
) -> Result<()> {
    // Default no-op to keep core crate free of Light SDK dependency.
    Ok(())
}

#[cfg(feature = "compressed-mint-light")]
impl From<light_compressed_token_sdk::TokenSdkError> for crate::error::Error {
    fn from(err: light_compressed_token_sdk::TokenSdkError) -> Self {
        crate::error::Error::from(crate::error::ErrorCode::AccountDidNotSerialize)
            .with_account_name(format!("TokenSdkError: {:?}", err))
    }
}

#[cfg(feature = "compressed-mint-light")]
/// Context for batched compressed operations (CPDAs and CMints)
pub struct CompressedBatchContext {
    pub cpda_indices: Vec<u32>,
    pub cmint_index: Option<u32>,
}

#[cfg(feature = "compressed-mint-light")]
/// Finalizes compressed operations including CPDAs and CMints in a single batched CPI.
pub fn finalize_compressed_batch<'info>(
    batch_context: &CompressedBatchContext,
    mint: Option<&CMint<'info>>,
    mint_actions: Vec<MintAction>,
    remaining: &[AccountInfo<'info>],
    ix_data: &[u8],
) -> Result<()> {
    use crate::error::ErrorCode;
    use crate::solana_program::program::invoke_signed;
    use borsh::BorshDeserialize;
    use light_compressed_token_sdk::instructions::create_mint_action_cpi;
    use light_compressed_token_sdk::instructions::CreateMintInputs;
    use light_compressed_token_sdk::instructions::MintActionType;
    use light_sdk::cpi::{CompressedCpiContext, CpiAccountsSmall};
    use light_sdk::instruction::{borsh_compat::ValidityProof, PackedAddressTreeInfo};
    use light_sdk_types::CpiAccountsConfig;

    // Constants from the user's program
    // const POOL_LP_MINT_SEED: &[u8] = b"pool_lp_mint";
    // const AUTH_SEED: &[u8] = b"amm_authority";

    // Parse compression params from instruction data
    // Expected structure: InitializeCompressionParams at end of ix_data
    #[derive(BorshDeserialize)]
    struct InitializeCompressionParams {
        pub proof: ValidityProof,
        pub tree_info: PackedAddressTreeInfo,
    }

    // Try to deserialize compression params from the end of ix_data
    let compression_params = if ix_data.len() > 8 {
        // Skip the discriminator (8 bytes) and any other instruction params
        // This is a simplified approach - in production you'd need to know exact offset
        let mut slice = &ix_data[8..];
        InitializeCompressionParams::try_from_slice(&mut slice)
            .map_err(|_| ErrorCode::InvalidInstructionData)?
    } else {
        return Err(ErrorCode::InvalidInstructionData.into());
    };

    // Build CpiAccountsSmall from remaining accounts
    // Expected order based on user's manual implementation:
    // 0: compressed_token_program
    // 1: compressed_token_program_cpi_authority
    // 2: authority (pool authority)
    // 3: payer
    // 4: pool_state (if needed for seeds)
    // 5: system_program
    // 6+: CPDA accounts (if any)
    // N+: recipient accounts for mint_to actions

    if remaining.len() < 6 {
        return Err(ErrorCode::AccountNotEnoughKeys.into());
    }

    let compressed_token_program = &remaining[0];
    let compressed_token_program_cpi_authority = &remaining[1];
    let authority = &remaining[2];
    let payer = &remaining[3];
    let pool_state = &remaining[4];
    let system_program = &remaining[5];

    // Build CPI accounts
    let cpi_accounts = CpiAccountsSmall {
        light_system_program: compressed_token_program.clone(),
        cpi_signer: compressed_token_program_cpi_authority.clone(),
        remaining_accounts: &[],
    };

    // Start building the CPI context chain
    let mut cpi_context: Option<CompressedCpiContext> = None;

    // First, add all CPDA compress operations
    for (i, cpda_index) in batch_context.cpda_indices.iter().enumerate() {
        // For CPDAs, we're just marking them for compression
        // The actual PDA account info should be in remaining_accounts
        let cpda_account_index = 6 + i; // CPDAs start after the standard accounts
        if cpda_account_index >= remaining.len() {
            return Err(ErrorCode::AccountNotEnoughKeys.into());
        }

        // Build compress PDA context
        cpi_context = Some(if let Some(prev_context) = cpi_context {
            // Chain with previous context
            CompressedCpiContext::cpi_compress_pda(&cpi_accounts, &prev_context, *cpda_index)
        } else {
            // First context in chain
            CompressedCpiContext::first_cpi_compress_pda(&cpi_accounts, *cpda_index)
        });
    }

    // Then add CMint operation if present
    if let (Some(mint), Some(cmint_index)) = (mint, batch_context.cmint_index) {
        // Get the mint signer info
        let lp_mint_signer = mint.as_ref();

        // Derive the SPL mint address from the signer
        let (lp_mint, _) = Pubkey::find_program_address(
            &[lp_mint_signer.key.as_ref()],
            &light_compressed_token_sdk::ID,
        );

        // Derive the compressed mint address
        let compressed_mint_address =
            light_compressed_token_sdk::instructions::derive_compressed_mint_from_spl_mint(
                &lp_mint,
            );

        // Build mint inputs from cmint constraints
        let mint_inputs = CreateMintInputs {
            authority: mint.authority.unwrap_or(*authority.key),
            decimals: mint.decimals.unwrap_or(9),
            mint_seed: Some(lp_mint_signer.key.to_bytes()),
            mint_creation_index: Some(cmint_index),
            proof: Some(compression_params.proof.clone()),
        };

        // Convert queued actions to MintActionType
        let mut mint_action_types = vec![];
        for action in &mint_actions {
            mint_action_types.push(MintActionType::MintToCToken {
                recipient: action.recipient,
                amount: action.amount,
            });
        }

        // Add mint to context chain
        cpi_context = Some(if let Some(prev_context) = cpi_context {
            // Chain with previous context (CPDAs)
            CompressedCpiContext::cpi_create_mint(
                &cpi_accounts,
                &mint_inputs,
                &mint_action_types,
                &prev_context,
                cmint_index,
            )
        } else {
            // Only CMint, no CPDAs
            CompressedCpiContext::first_cpi_create_mint(
                &cpi_accounts,
                &mint_inputs,
                &mint_action_types,
                cmint_index,
            )
        });
    }

    // If we have any compressed operations, execute the batched CPI
    if let Some(final_context) = cpi_context {
        // Build the instruction using the final chained context
        let ix = if mint.is_some() && !mint_actions.is_empty() {
            // If we have mint actions, use create_mint_action_cpi
            let mint_action_types: Vec<MintActionType> = mint_actions
                .iter()
                .map(|action| MintActionType::MintToCToken {
                    recipient: action.recipient,
                    amount: action.amount,
                })
                .collect();

            // Get compressed mint address (we know mint exists here)
            let lp_mint_signer = mint.unwrap().as_ref();
            let (lp_mint, _) = Pubkey::find_program_address(
                &[lp_mint_signer.key.as_ref()],
                &light_compressed_token_sdk::ID,
            );
            let compressed_mint_address =
                light_compressed_token_sdk::instructions::derive_compressed_mint_from_spl_mint(
                    &lp_mint,
                );

            create_mint_action_cpi(
                &final_context,
                &compressed_mint_address,
                &mint_action_types,
                Some(compression_params.tree_info),
            )
        } else {
            // CPDAs only - use system program compress instruction
            light_sdk::instructions::create_compress_pda_cpi(
                &final_context,
                Some(compression_params.tree_info),
            )
        };

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

        account_infos.extend([payer.clone(), system_program.clone()]);

        // Add CPDA accounts
        let cpda_start_index = 6;
        for i in 0..batch_context.cpda_indices.len() {
            let cpda_index = cpda_start_index + i;
            if cpda_index >= remaining.len() {
                return Err(ErrorCode::AccountNotEnoughKeys.into());
            }
            account_infos.push(remaining[cpda_index].clone());
        }

        // Add recipient accounts for mint_to actions
        if mint.is_some() {
            let recipient_start = cpda_start_index + batch_context.cpda_indices.len();
            for (i, action) in mint_actions.iter().enumerate() {
                let recipient_index = recipient_start + i;
                if recipient_index >= remaining.len() {
                    return Err(ErrorCode::AccountNotEnoughKeys.into());
                }
                let recipient_account = &remaining[recipient_index];
                // Verify the recipient key matches
                if recipient_account.key != &action.recipient {
                    return Err(ErrorCode::ConstraintAddress.into());
                }
                account_infos.push(recipient_account.clone());
            }
        }

        // Prepare signer seeds from cmint constraints
        let mut all_signer_seeds = vec![];

        if let Some(mint) = mint {
            // Add mint signer seeds if provided
            if let (Some(seeds), Some(bump)) = (&mint.mint_signer_seeds, mint.mint_signer_bump) {
                let mut mint_seeds: Vec<&[u8]> = seeds.iter().map(|s| s.as_slice()).collect();
                mint_seeds.push(&[bump]);
                all_signer_seeds.push(mint_seeds);
            }

            // Add program authority seeds if provided
            if let (Some(seeds), Some(bump)) =
                (&mint.program_authority_seeds, mint.program_authority_bump)
            {
                let mut auth_seeds: Vec<&[u8]> = seeds.iter().map(|s| s.as_slice()).collect();
                auth_seeds.push(&[bump]);
                all_signer_seeds.push(auth_seeds);
            }
        }

        // Convert to the format invoke_signed expects
        let signer_seeds: Vec<&[&[u8]]> = all_signer_seeds.iter().map(|s| s.as_slice()).collect();

        // Execute the CPI with signer seeds
        invoke_signed(&ix, &account_infos, &signer_seeds)?;
    }

    Ok(())
}

#[cfg(feature = "compressed-mint-light")]
/// Finalizes a compressed mint by executing any queued actions in a single batch.
/// This is a compatibility wrapper that calls the batched version.
pub fn finalize_cmint<'info>(
    mint: &CMint<'info>,
    actions: Vec<MintAction>,
    remaining: &[AccountInfo<'info>],
    ix_data: &[u8],
) -> Result<()> {
    // For backward compatibility, assume CMint index is 0 when called directly
    let batch_context = CompressedBatchContext {
        cpda_indices: vec![],
        cmint_index: Some(0),
    };

    finalize_compressed_batch(&batch_context, Some(mint), actions, remaining, ix_data)
}

// Stub for prepare_accounts_for_empty_compression_on_init
// This is for the 'compressible' flag (prepare for later compression, no auto-close)
#[cfg(feature = "compressed-mint-light")]
pub fn prepare_accounts_for_empty_compression_on_init<T>(
    _accounts: &[&T],
    _compressed_addresses: &[[u8; 32]],
    _new_address_params: &[light_sdk::instruction::NewAddressParams],
    _output_state_tree_indices: &[u8],
    _cpi_accounts: &light_sdk::cpi::CpiAccountsSmall,
    _address_space: &[Pubkey],
    _rent_recipient: &AccountInfo,
) -> Result<Vec<light_sdk::cpi::CompressedAccountInfo>> {
    // TODO: Implement prepare_accounts_for_empty_compression_on_init
    // This should prepare accounts for future compression without immediately compressing
    unimplemented!("prepare_accounts_for_empty_compression_on_init not yet implemented. Use compress_on_init for now.")
}
