use crate::accounts_codegen::constraints::OptionalCheckScope;
use crate::codegen::accounts::{generics, ParsedGenerics};
use crate::{AccountField, AccountsStruct, Ty};
use quote::quote;

// Generates the `Exit` trait implementation.
pub fn generate(accs: &AccountsStruct) -> proc_macro2::TokenStream {
    let name = &accs.ident;
    let ParsedGenerics {
        combined_generics,
        trait_generics,
        struct_generics,
        where_clause,
    } = generics(accs);

    let on_save: Vec<proc_macro2::TokenStream> = accs
        .fields
        .iter()
        .map(|af: &AccountField| match af {
            AccountField::CompositeField(s) => {
                let name = &s.ident;
                let name_str = name.to_string();
                quote! {
                    anchor_lang::AccountsExit::exit(&self.#name, program_id)
                        .map_err(|e| e.with_account_name(#name_str))?;
                }
            }
            AccountField::Field(f) => {
                let ident = &f.ident;
                let name_str = ident.to_string();
                if f.constraints.is_close() {
                    let close_target = &f.constraints.close.as_ref().unwrap().sol_dest;
                    let close_target_optional_check =
                        OptionalCheckScope::new(accs).generate_check(close_target);

                    quote! {
                        {
                            let #close_target = &self.#close_target;
                            #close_target_optional_check
                            anchor_lang::AccountsClose::close(
                                &self.#ident,
                                #close_target.to_account_info(),
                            ).map_err(|e| e.with_account_name(#name_str))?;
                        }
                    }
                } else {
                    match f.constraints.is_mutable() {
                        false => quote! {},
                        true => match &f.ty {
                            // `LazyAccount` is special because it has a custom `exit` method.
                            Ty::LazyAccount(_) => quote! {
                                self.#ident.exit(program_id)
                                    .map_err(|e| e.with_account_name(#name_str))?;
                            },
                            _ => quote! {
                                anchor_lang::AccountsExit::exit(&self.#ident, program_id)
                                    .map_err(|e| e.with_account_name(#name_str))?;
                            },
                        },
                    }
                }
            }
        })
        .collect();

    // Scan for compressible Account fields and CMints to assign indices
    let mut compressible_fields = Vec::new();
    let mut compress_on_init_fields = Vec::new();
    let mut cmint_field = None;
    
    for af in &accs.fields {
        if let AccountField::Field(f) = af {
            // Check if this is a compressible Account
            if matches!(&f.ty, Ty::Account(_)) {
                if let Some(init) = &f.constraints.init {
                    if init.compressible {
                        compressible_fields.push(&f.ident);
                    } else if init.compress_on_init {
                        compress_on_init_fields.push(&f.ident);
                    }
                }
            } else if matches!(&f.ty, Ty::CMint(_)) {
                if cmint_field.is_none() {
                    cmint_field = Some(&f.ident);
                }
            }
        }
    }
    
    // Check for conflicting compression modes
    if !compressible_fields.is_empty() && !compress_on_init_fields.is_empty() {
        return syn::Error::new(
            proc_macro2::Span::call_site(),
            "Cannot mix 'compressible' and 'compress_on_init' flags on different accounts. Use either all 'compressible' or all 'compress_on_init'."
        ).to_compile_error();
    }
    
    // Determine which fields to process
    let fields_to_process = if !compress_on_init_fields.is_empty() {
        &compress_on_init_fields
    } else if !compressible_fields.is_empty() {
        &compressible_fields
    } else {
        &Vec::new()
    };
    
    // Generate the batched finalize for compressed operations
    let compressed_finalize = if !fields_to_process.is_empty() || cmint_field.is_some() {
        use quote::format_ident;

        // Build per-compressible account blocks
        let mut compress_blocks: Vec<proc_macro2::TokenStream> = Vec::new();
        let mut new_address_params_idents: Vec<proc_macro2::TokenStream> = Vec::new();

        for (index, ident) in fields_to_process.iter().enumerate() {
            let idx_lit = index as u8;
            // Build the corresponding "<field>_address_tree_info" identifier from ix args
            let info_ident = format_ident!("{}_address_tree_info", ident);

            // Resolve the account type for generic prepare function
            let acc_ty_path = match accs
                .fields
                .iter()
                .find_map(|af| match af {
                    AccountField::Field(f) if &f.ident == *ident => {
                        match &f.ty {
                            Ty::Account(account_ty) => Some(account_ty.account_type_path.clone()),
                            _ => None,
                        }
                    }
                    _ => None,
                }) {
                Some(p) => p,
                None => panic!("Invariant: compressible Account type path not found"),
            };

            // Handle boxed vs unboxed Account
            let acc_expr = match accs
                .fields
                .iter()
                .find_map(|af| match af {
                    AccountField::Field(f) if &f.ident == *ident => {
                        match &f.ty {
                            Ty::Account(account_ty) => {
                                if account_ty.boxed { Some(quote! { &*self.#ident }) } else { Some(quote! { &self.#ident }) }
                            }
                            _ => None,
                        }
                    }
                    _ => None,
                }) {
                Some(expr) => expr,
                None => panic!("Invariant: compressible Account expr not found"),
            };

            let new_addr_params_ident = format_ident!("{}_new_address_params", ident);
            let compressed_address_ident = format_ident!("{}_compressed_address", ident);
            let compressed_infos_ident = format_ident!("{}_compressed_infos", ident);
            new_address_params_idents.push(quote! { #new_addr_params_ident });

            // Different preparation based on compression mode
            let prepare_fn = if !compress_on_init_fields.is_empty() {
                quote! { prepare_accounts_for_compression_on_init }
            } else {
                quote! { prepare_accounts_for_empty_compression_on_init }
            };
            
            compress_blocks.push(quote! {
                // Build new address params for #ident
                let #new_addr_params_ident = compression_params.#info_ident
                    .into_new_address_params_assigned_packed(
                        self.#ident.key().to_bytes(),
                        true,
                        Some(#idx_lit),
                    );

                // Derive compressed address for #ident
                let #compressed_address_ident = light_compressed_account::address::derive_address(
                    &self.#ident.key().to_bytes(),
                    &cpi_accounts
                        .get_tree_address(#new_addr_params_ident.address_merkle_tree_account_index)
                        .unwrap()
                        .key
                        .to_bytes(),
                    &crate::ID.to_bytes(),
                );

                // Prepare accounts for compression for #ident
                let #compressed_infos_ident = light_sdk::compressible::#prepare_fn::<#acc_ty_path>(
                    &[#acc_expr],
                    &[#compressed_address_ident],
                    &[#new_addr_params_ident],
                    &[compression_params.output_state_tree_index],
                    &cpi_accounts,
                    &address_space,
                    &self.rent_recipient,
                )?;
                all_compressed_infos.extend(#compressed_infos_ident);
            });
        }

        let compressible_count = fields_to_process.len() as u32;
        let cmint_index = if cmint_field.is_some() {
            quote! { Some(#compressible_count) }
        } else {
            quote! { None }
        };

        let mint_ident = cmint_field.cloned();

        // Build mint actions extraction and mint creation block
        let cmint_block = if let Some(cmint_ident) = mint_ident {
            quote! {
                // Drain queued CMint actions
                let __mint_actions = self.#cmint_ident.take_actions();

                if !__mint_actions.is_empty() {
                    // Tree accounts indices
                    let output_state_queue_idx: u8 = 0;
                    let address_tree_idx: u8 = 1;
                    let output_state_queue = *cpi_accounts.tree_accounts().unwrap()[output_state_queue_idx as usize].key;
                    let address_tree_pubkey = *cpi_accounts.tree_accounts().unwrap()[address_tree_idx as usize].key;

                    // Derive compressed mint address from SPL mint addr (self.#cmint_ident is used as SPL mint key)
                    let mint_compressed_address = light_compressed_token_sdk::instructions::derive_compressed_mint_from_spl_mint(
                        &self.#cmint_ident.key(),
                        &address_tree_pubkey,
                    );

                    // Build compressed mint with context
                    let compressed_mint_with_context = light_ctoken_types::instructions::mint_action::CompressedMintWithContext::new(
                        mint_compressed_address,
                        compression_params.lp_mint_address_tree_info.root_index,
                        self.#cmint_ident.decimals.unwrap_or(9),
                        Some(self.authority.key().into()),
                        Some(self.authority.key().into()),
                        self.#cmint_ident.key().into(),
                    );

                    // Build actions
                    let actions: Vec<light_compressed_token_sdk::instructions::MintActionType> = __mint_actions
                        .iter()
                        .map(|a| light_compressed_token_sdk::instructions::MintActionType::MintToCToken {
                            account: a.recipient,
                            amount: a.amount,
                        })
                        .collect();

                    // Create mint inputs
                    let inputs = light_compressed_token_sdk::instructions::CreateMintInputs {
                        compressed_mint_inputs: compressed_mint_with_context,
                        mint_seed: self.#cmint_ident.key(),
                        mint_bump: compression_params.lp_mint_bump,
                        authority: self.authority.key().into(),
                        payer: self.creator.key(),
                        proof: compression_params.proof.0.map(|p| light_compressed_token_sdk::CompressedProof::from(p)),
                        address_tree: address_tree_pubkey,
                        output_queue: output_state_queue,
                        actions,
                    };

                    // Build instruction
                    let mint_action_instruction: anchor_lang::solana_program::instruction::Instruction =
                        light_compressed_token_sdk::instructions::create_mint_action_cpi(
                            light_compressed_token_sdk::instructions::MintActionInputs::new_create_mint(inputs),
                            Some(light_ctoken_types::instructions::mint_action::CpiContext::last_cpi_create_mint(
                                address_tree_idx,
                                output_state_queue_idx,
                                #compressible_count,
                            )),
                            Some(cpi_accounts.cpi_context().unwrap().key()),
                        )?;

                    // Accounts for CPI
                    let mut account_infos = cpi_accounts.to_account_infos();
                    account_infos.extend([
                        self.compressed_token_program_cpi_authority.clone(),
                        self.compressed_token_program.clone(),
                        self.authority.clone(),
                        self.creator.clone(),
                        // recipients
                        self.lp_vault.clone(),
                        self.creator_lp_token.clone(),
                    ]);

                    // Signer seeds
                    let mut signer_seeds: Vec<&[&[u8]]> = Vec::new();
                    if let (Some(seeds), Some(bump)) = (&self.#cmint_ident.mint_signer_seeds, self.#cmint_ident.mint_signer_bump) {
                        let mut s: Vec<&[u8]> = seeds.iter().map(|v| v.as_slice()).collect();
                        s.push(&[bump]);
                        signer_seeds.push(Box::leak(s.into_boxed_slice()));
                    }
                    if let (Some(seeds), Some(bump)) = (&self.#cmint_ident.program_authority_seeds, self.#cmint_ident.program_authority_bump) {
                        let mut s: Vec<&[u8]> = seeds.iter().map(|v| v.as_slice()).collect();
                        s.push(&[bump]);
                        signer_seeds.push(Box::leak(s.into_boxed_slice()));
                    }

                    anchor_lang::solana_program::program::invoke_signed(
                        &mint_action_instruction,
                        &account_infos,
                        &signer_seeds,
                    )?;
                }
            }
        } else {
            quote! {}
        };

        // Build the full compressed finalize block
        quote! {
            #[cfg(feature = "compressed-mint")]
            {
                use borsh::BorshDeserialize;

                // Build CPI accounts with CPI context signer
                let cpi_accounts = light_sdk::cpi::CpiAccountsSmall::new_with_config(
                    &self.creator,
                    _remaining,
                    light_sdk_types::CpiAccountsConfig::new_with_cpi_context(crate::LIGHT_CPI_SIGNER),
                );

                // Load compression config
                let compression_config = light_sdk::compressible::CompressibleConfig::load_checked(&self.compression_config, &crate::ID)?;
                let address_space = compression_config.address_space;

                // Parse compression params from ix_data
                #[derive(BorshDeserialize)]
                struct InitializeCompressionParams {
                    pub pool_address_tree_info: light_sdk::instruction::PackedAddressTreeInfo,
                    pub observation_address_tree_info: light_sdk::instruction::PackedAddressTreeInfo,
                    pub lp_mint_address_tree_info: light_sdk::instruction::PackedAddressTreeInfo,
                    pub lp_mint_bump: u8,
                    pub creator_lp_token_bump: u8,
                    pub proof: light_sdk::instruction::borsh_compat::ValidityProof,
                    pub output_state_tree_index: u8,
                }
                let compression_params = {
                    let mut slice = &_ix_data[8..];
                    InitializeCompressionParams::try_from_slice(&mut slice)
                        .map_err(|_| anchor_lang::error::ErrorCode::InvalidInstructionData)?
                };

                // Collect compressed infos for all compressible accounts
                let mut all_compressed_infos = Vec::new();
                #(#compress_blocks)*

                // Invoke ONE system-program batched CPI for all CPDAs
                let cpi_inputs = light_sdk::cpi::CpiInputs::new_first_cpi(
                    all_compressed_infos,
                    vec![#(#new_address_params_idents),*],
                );
                let cpi_context = cpi_accounts.cpi_context().unwrap();
                let cpi_context_accounts = light_sdk_types::cpi_context_write::CpiContextWriteAccounts {
                    fee_payer: cpi_accounts.fee_payer(),
                    authority: cpi_accounts.authority().unwrap(),
                    cpi_context,
                    cpi_signer: crate::LIGHT_CPI_SIGNER,
                };
                cpi_inputs.invoke_light_system_program_cpi_context(cpi_context_accounts)?;

                // Then chain CMint creation and mint_to actions as last CPI
                #cmint_block
            }
        }
    } else {
        quote! {}
    };
    
    // Generate auto-close for compress_on_init fields
    let auto_close_block = if !compress_on_init_fields.is_empty() {
        let close_statements: Vec<_> = compress_on_init_fields.iter().map(|ident| {
            quote! {
                self.#ident.close(self.rent_recipient.clone())?;
            }
        }).collect();
        
        quote! {
            // Auto-close accounts marked with compress_on_init
            #(#close_statements)*
        }
    } else {
        quote! {}
    };
    
    // Regular finalize for non-compressed accounts
    let on_finalize: Vec<proc_macro2::TokenStream> = accs
        .fields
        .iter()
        .map(|af: &AccountField| match af {
            AccountField::CompositeField(s) => {
                let name = &s.ident;
                let name_str = name.to_string();
                quote! {
                    // Composite fields do not implement finalize by default.
                }
            }
            AccountField::Field(f) => {
                let ident = &f.ident;
                let name_str = ident.to_string();
                match &f.ty {
                    Ty::AccountInfo | Ty::UncheckedAccount | Ty::Signer | 
                    Ty::SystemAccount | Ty::ProgramData | Ty::CMint(_) => {
                        // Skip these - CMint handled in batched finalize
                        quote! {}
                    }
                    Ty::Account(_) if f.constraints.init.as_ref().map(|i| i.compressible).unwrap_or(false) => {
                        // Skip compressible accounts - handled in batched finalize
                        quote! {}
                    }
                    _ => quote! {
                        anchor_lang::AccountsFinalize::finalize(&self.#ident, program_id, _remaining, _ix_data)
                            .map_err(|e| e.with_account_name(#name_str))?;
                    },
                }
            }
        })
        .collect();
    quote! {
        #[automatically_derived]
        impl<#combined_generics> anchor_lang::AccountsFinalize<#trait_generics> for #name<#struct_generics> #where_clause{
            fn finalize(
                &self,
                program_id: &anchor_lang::solana_program::pubkey::Pubkey,
                _remaining: &[anchor_lang::solana_program::account_info::AccountInfo<#trait_generics>],
                _ix_data: &[u8],
            ) -> anchor_lang::Result<()> {
                // Finalize all nested/composite fields first, then each field.
                #(#on_finalize)*
                // Run compressed operations (CPDAs + CMint) in a single batched CPI
                #compressed_finalize
                #auto_close_block
                Ok(())
            }
        }

        #[automatically_derived]
        impl<#combined_generics> anchor_lang::AccountsExit<#trait_generics> for #name<#struct_generics> #where_clause{
            fn exit(&self, program_id: &anchor_lang::solana_program::pubkey::Pubkey) -> anchor_lang::Result<()> {
                #(#on_save)*
                Ok(())
            }
        }
    }
}
