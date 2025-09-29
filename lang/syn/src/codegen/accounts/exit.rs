use crate::accounts_codegen::constraints::OptionalCheckScope;
use crate::codegen::accounts::{bumps, generics, ParsedGenerics};
use crate::{AccountField, AccountsStruct, Ty};
use quote::quote;
use syn::Expr;

// Generates the `Exit` trait implementation.
pub fn generate(accs: &AccountsStruct) -> proc_macro2::TokenStream {
    let name = &accs.ident;
    let bumps_struct_name = bumps::generate_bumps_name(&accs.ident);
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

    // Scan for CPDA Account fields and CMints to assign indices
    let mut cpda_fields = Vec::new();
    let mut cmint_field = None;
    
    for af in &accs.fields {
        if let AccountField::Field(f) = af {
            // Check if this is a CPDA Account
            // TODO: add support for compressible flag
            if let Some(cpda) = &f.constraints.cpda {
                // if cpda.compressible {
                //     panic!("CPDA Account fields with compressible flag are not yet supported");
                // }
                if cpda.compress_on_init {
                    cpda_fields.push(&f.ident);
                }
            } else if matches!(&f.ty, Ty::CMint(_)) {
                if cmint_field.is_none() {
                    cmint_field = Some(&f.ident);
                }
            }
        }
    }
    
    // Use CPDA fields for processing
    let fields_to_process = &cpda_fields;
    
    // Find the first Signer field to use as fee payer
    let fee_payer_ident = accs.fields.iter().find_map(|af| {
        if let AccountField::Field(f) = af {
            if matches!(&f.ty, Ty::Signer) {
                Some(&f.ident)
            } else {
                None
            }
        } else {
            None
        }
    });
    
    // Find required fields by name
    let mut required_fields = std::collections::HashMap::new();
    for af in &accs.fields {
        if let AccountField::Field(f) = af {
            let name = f.ident.to_string();
            match name.as_str() {
                "compression_config" | "rent_recipient" | 
                "compressed_token_program_cpi_authority" | 
                "compressed_token_program" | "authority" => {
                    required_fields.insert(name, &f.ident);
                }
                _ => {}
            }
        }
    }
    
    let has_all_required = required_fields.contains_key("compression_config") &&
                          required_fields.contains_key("compressed_token_program_cpi_authority") &&
                          required_fields.contains_key("compressed_token_program");
    
    // Generate instruction deserialization for finalize (same as try_accounts)
    let ix_de = match &accs.instruction_api {
        None => quote! {},
        Some(ix_api) => {
            let strct_inner = &ix_api;
            // Collect field identifiers from the instruction declaration
            let field_idents: Vec<proc_macro2::TokenStream> = ix_api
                .iter()
                .map(|expr: &Expr| match expr {
                    Expr::Type(expr_type) => {
                        let field = &expr_type.expr;
                        quote! { #field }
                    }
                    _ => panic!("Invalid instruction declaration"),
                })
                .collect();
            // Generate re-bindings to move fields out of __args explicitly
            let field_rebinds: Vec<proc_macro2::TokenStream> = ix_api
                .iter()
                .map(|expr: &Expr| match expr {
                    Expr::Type(expr_type) => {
                        let field = &expr_type.expr;
                        quote! { let #field = __args.#field; }
                    }
                    _ => panic!("Invalid instruction declaration"),
                })
                .collect();
            quote! {
                let mut __ix_data = _ix_data;
                #[derive(anchor_lang::AnchorSerialize, anchor_lang::AnchorDeserialize)]
                struct __Args {
                    #strct_inner
                }
                let __args: __Args = __Args::deserialize(&mut __ix_data)
                    .map_err(|_| anchor_lang::error::ErrorCode::InstructionDidNotDeserialize)?;
                // Move fields out of __args into local variables
                #(#field_rebinds)*
                // Prevent unused warning for the holder
                let _ = __args;
            }
        }
    };
    
    // Generate the batched finalize for compressed operations
    // Get the payer from the CMint if it exists
    let cmint_payer = cmint_field.and_then(|cmint_ident| {
        accs.fields.iter().find_map(|af| {
            if let AccountField::Field(f) = af {
                if &f.ident == cmint_ident {
                    f.constraints.cmint.as_ref().and_then(|c| c.payer.as_ref())
                } else {
                    None
                }
            } else {
                None
            }
        })
    });
    
    let compressed_finalize = if (!fields_to_process.is_empty() || cmint_field.is_some()) 
        && (cmint_payer.is_some() || fee_payer_ident.is_some())
        && has_all_required {
        use quote::format_ident;
        
        
        let fee_payer = if let Some(payer_expr) = cmint_payer {
            quote! { #payer_expr }
        } else {
            let ident = fee_payer_ident.unwrap();
            quote! { #ident }
        };
        let compression_config = required_fields.get("compression_config").unwrap();
        let compressed_token_program_cpi_authority = required_fields.get("compressed_token_program_cpi_authority").unwrap();
        let compressed_token_program = required_fields.get("compressed_token_program").unwrap();

        // Build per-compressible account blocks
        let mut compress_blocks: Vec<proc_macro2::TokenStream> = Vec::new();
        let mut new_address_params_idents: Vec<proc_macro2::TokenStream> = Vec::new();

        for (index, ident) in fields_to_process.iter().enumerate() {
            let idx_lit = index as u8;

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

            // Use prepare_accounts_for_compression_on_init for CPDA fields
            let prepare_fn = quote! { prepare_accounts_for_compression_on_init };
            
            // Get constraint expressions from CPDA including authority
            let (address_tree_info_expr, proof_expr, output_tree_expr, cpda_authority_expr) = accs.fields.iter().find_map(|af| {
                if let AccountField::Field(field) = af {
                    if field.ident == **ident {
                        if let Some(cpda) = &field.constraints.cpda {
                            let authority = cpda.authority.as_ref().map(|e| quote! { #e });
                            return Some((
                                cpda.address_tree_info.as_ref().map(|e| quote! { #e }).unwrap_or_else(|| panic!("CPDA address_tree_info is required")),
                                cpda.proof.as_ref().map(|e| quote! { #e }).unwrap_or_else(|| panic!("CPDA proof is required")),
                                cpda.output_state_tree_index.as_ref().map(|e| quote! { #e }).unwrap_or_else(|| panic!("CPDA output_state_tree_index is required")),
                                authority
                            ));
                        }
                    }
                }
                None
            }).unwrap_or_else(|| panic!("CPDA constraints not found for account"));
            
            let tree_info_ident = format_ident!("{}_tree_info", ident);
            compress_blocks.push(quote! {
                // Build new address params for #ident using constraint expression (move, avoid clone)
                let #tree_info_ident = #address_tree_info_expr;
                let #new_addr_params_ident = #tree_info_ident
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
                    &[#output_tree_expr],
                    &cpi_accounts,
                    &address_space,
                    &self.rent_recipient,
                )?;
                all_compressed_infos.extend(#compressed_infos_ident);
            });
        }

        let compressible_count = fields_to_process.len() as u8;
        let cmint_index = if cmint_field.is_some() {
            quote! { Some(#compressible_count) }
        } else {
            quote! { None }
        };

        let mint_ident = cmint_field.cloned();

        // Build mint actions extraction and mint creation block
        let cmint_block = if let Some(cmint_ident) = mint_ident {
            // Get CMint constraints for this field
            let cmint_constraints = accs.fields.iter().find_map(|af| {
                if let AccountField::Field(f) = af {
                    if f.ident.to_string() == cmint_ident.to_string() {
                        return f.constraints.cmint.as_ref();
                    }
                }
                None
            });
           
            
            // Build expressions from constraints or fallback to instruction data
            let cmint_proof_expr = cmint_constraints
                .and_then(|c| c.proof.as_ref())
                .map(|e| quote! { #e })
                .unwrap_or_else(|| quote! { panic!("CMint proof constraint is required") });
                
            let cmint_address_tree_info_expr = cmint_constraints
                .and_then(|c| c.address_tree_info.as_ref())
                .map(|e| quote! { #e })
                .unwrap_or_else(|| quote! { panic!("CMint address_tree_info constraint is required") });
                
            let cmint_output_tree_expr = cmint_constraints
                .and_then(|c| c.output_state_tree_index.as_ref())
                .map(|e| quote! { #e })
                .unwrap_or_else(|| quote! { panic!("CMint output_state_tree_index constraint is required") });
                
            let cmint_authority_expr = cmint_constraints
                .and_then(|c| c.authority.as_ref())
                .map(|e| quote! { {
                    let __auth = &self.#e;
                    __auth.key() 
                }})
                .unwrap_or_else(|| quote! { self.authority.key() });
                
            let cmint_payer_expr = cmint_constraints
                .and_then(|c| c.payer.as_ref())
                .map(|e| quote! { {
                    let __payer = &self.#e;
                    __payer.key()
                }})
                .unwrap_or_else(|| {
                    if let Some(fee_payer) = fee_payer_ident {
                        quote! { self.#fee_payer.key() }
                    } else {
                        quote! { self.creator.key() }
                    }
                });
                
            // Get the mint_signer field name from the constraint at compile time
            let mint_signer_bump_expr = cmint_constraints
                .and_then(|c| c.mint_signer.as_ref())
                .map(|signer_field| quote! { _bumps.#signer_field })
                .unwrap_or_else(|| quote! { panic!("mint_signer constraint is required for CMint") });
            
            // Check if we have compressed PDAs that need authority signing
            let has_compress_on_init = !fields_to_process.is_empty();
            
            // Collect all unique authorities and their seeds
            // Each CPDA MUST have an explicit authority when using compress_on_init
            let mut authority_expr = None;
            
            // Check that all compress_on_init CPDAs have an explicit authority
            for ident in fields_to_process.iter() {
                if let Some(cpda) = accs.fields.iter().find_map(|af| {
                    if let AccountField::Field(field) = af {
                        if &field.ident == *ident {
                            return field.constraints.cpda.as_ref();
                        }
                    }
                    None
                }) {
                    if cpda.authority.is_none() {
                        // Compile-time panic if compress_on_init without authority
                        panic!(
                            "CPDA field '{}' has compress_on_init but no cpda::authority constraint. \
                            All compress_on_init CPDAs must explicitly specify their authority.",
                            ident
                        );
                    }
                    // Use the first authority we find (they should all be the same in practice)
                    if authority_expr.is_none() {
                        authority_expr = cpda.authority.as_ref();
                    }
                }
            }
            
            // Build the authority seeds extraction logic
            let authority_seeds_setup = if let Some(auth_expr) = authority_expr {
                // The auth_expr is an expression like "authority" 
                // We need to find the corresponding field and extract its seeds
                // For now, let's try to match by converting the expression to a string
                let auth_field = accs.fields.iter().find_map(|af| {
                    if let AccountField::Field(f) = af {
                        // Simple heuristic: if the field name matches the start of the expression
                        // This handles both "authority" and "self.authority" cases
                        let field_name_str = f.ident.to_string();
                        let expr_str = quote! { #auth_expr }.to_string();
                        if expr_str.contains(&field_name_str) {
                            Some(f)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                });
                
                if let Some(field) = auth_field {
                    if let Some(seeds_group) = &field.constraints.seeds {
                        let seeds = &seeds_group.seeds;
                        let seed_exprs: Vec<_> = seeds.iter().map(|s| {
                            quote! { #s.as_ref() }
                        }).collect();
                        
                        quote! {
                            // Build authority seeds from the authority PDA definition
                            let auth_bump = _bumps.#auth_expr;
                            auth_bump_array = [auth_bump];
                            let mut auth_seeds_for_cpi: Vec<&[u8]> = vec![#(#seed_exprs),*];
                            auth_seeds_for_cpi.push(&auth_bump_array);
                        }
                    } else {
                        quote! {
                            panic!("Authority field must be a PDA with seeds when compress_on_init PDAs exist")
                        }
                    }
                } else {
                    // Fallback: assume it's the standard authority with AUTH_SEED
                    quote! {
                        // Build authority seeds (fallback to AUTH_SEED pattern)
                        let auth_bump = _bumps.#auth_expr;
                        auth_bump_array = [auth_bump];
                        let mut auth_seeds_for_cpi: Vec<&[u8]> = vec![crate::AUTH_SEED.as_bytes()];
                        auth_seeds_for_cpi.push(&auth_bump_array);
                    }
                }
            } else {
                quote! {
                    // No compress_on_init PDAs, no authority seeds needed
                    let auth_seeds_for_cpi: Vec<&[u8]> = vec![];
                }
            };
            
            // Find the bump for the CMint PDA at runtime
            // The CMint account is a PDA derived from [b"compressed_mint", mint_signer]
            quote! {
                // Drain queued CMint actions - take ownership to avoid lifetime issues
                let __mint_actions = self.#cmint_ident.take_actions();
                if !__mint_actions.is_empty() {
                    // Find the bump for the compressed mint PDA
                    let compressed_mint_seed = b"compressed_mint";
                    let mint_signer_key = self.#cmint_ident
                        .mint_signer
                        .as_ref()
                        .map(|a| a.key())
                        .expect("mint_signer is required for CMint");
                    
                    let (expected_mint_address, mint_bump) = anchor_lang::solana_program::pubkey::Pubkey::find_program_address(
                        &[
                            compressed_mint_seed.as_ref(),
                            mint_signer_key.as_ref(),
                        ],
                        &self.#compressed_token_program.key(),  // CTOKEN_PROGRAM_ID
                    );
                    
                    // Verify the CMint account address matches the expected PDA
                    if self.#cmint_ident.key() != expected_mint_address {
                        panic!(
                            "CMint address mismatch: expected {:?}, got {:?}",
                            expected_mint_address,
                            self.#cmint_ident.key()
                        );
                    }
                    
                    // Get tree indices from constraints
                    let output_state_tree_index = #cmint_output_tree_expr;
                    let __address_tree_info = #cmint_address_tree_info_expr;
                    let address_tree_idx = __address_tree_info.address_merkle_tree_pubkey_index;
                    let address_queue_idx = __address_tree_info.address_queue_pubkey_index;
                    let root_index = __address_tree_info.root_index;
                    
                    // Get tree accounts using the indices
                    let output_state_queue = *cpi_accounts.tree_accounts().unwrap()[output_state_tree_index as usize].key;
                    let address_tree_pubkey = *cpi_accounts.tree_accounts().unwrap()[address_tree_idx as usize].key;

                    // Derive compressed mint address from SPL mint addr (self.#cmint_ident is used as SPL mint key)
                    let mint_compressed_address = light_compressed_token_sdk::instructions::derive_compressed_mint_from_spl_mint(
                        &self.#cmint_ident.key(),
                        &address_tree_pubkey,
                    );

                    // Build compressed mint with context
                    let compressed_mint_with_context = light_ctoken_types::instructions::mint_action::CompressedMintWithContext::new(
                        mint_compressed_address,
                        root_index,
                        self.#cmint_ident.decimals.unwrap_or(9),
                        Some(#cmint_authority_expr.into()),
                        Some(#cmint_authority_expr.into()),
                        self.#cmint_ident.key().into(),
                    );
                    // Build actions
                    let actions: Vec<light_compressed_token_sdk::instructions::MintActionType> = __mint_actions
                        .iter()
                        .map(|a| light_compressed_token_sdk::instructions::MintActionType::MintToCToken {
                            account: *a.recipient.key,
                            amount: a.amount,
                        })
                        .collect();
                    // Create mint inputs

                    let mint_signer = self.#cmint_ident
                            .mint_signer
                            .as_ref()
                            .map(|a| a.key())
                            .expect("mint_signer is required for cmint mint_seed");

                    let inputs = light_compressed_token_sdk::instructions::CreateMintInputs {
                        compressed_mint_inputs: compressed_mint_with_context,
                        // mint_seed: self.#cmint_ident.key(),
                        // Use the PDA mint_signer as the mint seed (must sign)
                        mint_seed: mint_signer,
                        mint_bump,  // Use the bump we just found from PDA derivation
                        authority: #cmint_authority_expr.into(),
                        payer: #cmint_payer_expr,
                        proof: #cmint_proof_expr.0.map(|p| light_compressed_token_sdk::CompressedProof::from(p)),
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
                                output_state_tree_index,
                                #compressible_count,
                            )),
                            Some(cpi_accounts.cpi_context().unwrap().key()),
                        ).map_err(|_| anchor_lang::error::ErrorCode::AccountDidNotSerialize)?;
                    // Accounts for CPI
                    let mut account_infos = cpi_accounts.to_account_infos();
                    account_infos.extend([
                        self.#compressed_token_program_cpi_authority.to_account_info(),
                        self.#compressed_token_program.to_account_info(),
                        self.authority.to_account_info(),
                    ]);
                    // Add mint_signer if provided
                    if let Some(mint_signer) = &self.#cmint_ident.mint_signer {
                        account_infos.push(mint_signer.clone());
                    } else {
                       panic!("mint_signer is required");
                    }
                    
                    account_infos.push(self.#fee_payer.to_account_info());
                    
                    // Add recipient accounts from mint actions
                    // The recipients are now AccountInfo objects, not just keys
                    for action in __mint_actions.iter() {
                        let recipient_info = &action.recipient;
                        let recipient_key = recipient_info.key;
                        
                        // Check if this account is already in account_infos (avoid duplicates)
                        let already_added = account_infos.iter().any(|ai| ai.key == recipient_key);
                        
                        if !already_added {
                            // Clone and add the recipient AccountInfo
                            account_infos.push(recipient_info.clone());
                        }
                    }
                    
           
                    // Build signer seeds without heap for outer groups
                    // Get mint_signer bump from __bumps based on the mint_signer field name
                    let mint_signer_bump = #mint_signer_bump_expr;
                    let mint_bump_array = [mint_signer_bump];
                    let mut mint_signer_seeds_vec: Vec<&[u8]> = Vec::new();
                    if let Some(seeds) = &self.#cmint_ident.mint_signer_seeds {
                        for seed in seeds.iter() { mint_signer_seeds_vec.push(seed.as_slice()); }
                        mint_signer_seeds_vec.push(&mint_bump_array);
                    }


                    // Build authority seeds for compressed PDA operations
                    let mut auth_seeds_vec: Vec<&[u8]> = Vec::new();
                    let has_compress_on_init = #has_compress_on_init;
                    
                    // Declare auth_bump_array at this scope so it lives long enough
                    let auth_bump_array;
                    
                    if has_compress_on_init {
                        // Build authority seeds from the actual PDA definition
                        #authority_seeds_setup
                        // Use the authority seeds we built from the actual PDA definition
                        auth_seeds_vec = auth_seeds_for_cpi;
                    } else if let Some(seeds) = &self.#cmint_ident.program_authority_seeds {
                        // Use explicitly provided program_authority_seeds if available
                        auth_bump_array = [self.#cmint_ident.program_authority_bump.unwrap_or_else(|| panic!("program_authority_bump is required"))];
                        for seed in seeds.iter() { auth_seeds_vec.push(seed.as_slice()); }
                        auth_seeds_vec.push(&auth_bump_array);
                    }
                    
                    match (mint_signer_seeds_vec.is_empty(), auth_seeds_vec.is_empty()) {
                        (false, false) => {
                            anchor_lang::solana_program::program::invoke_signed(
                                &mint_action_instruction,
                                &account_infos,
                                &[mint_signer_seeds_vec.as_slice(), auth_seeds_vec.as_slice()],
                            )?;
                        }
                        (false, true) => {
                            anchor_lang::solana_program::program::invoke_signed(
                                &mint_action_instruction,
                                &account_infos,
                                &[mint_signer_seeds_vec.as_slice()],
                            )?;
                        }
                        (true, false) => {
                            anchor_lang::solana_program::program::invoke_signed(
                                &mint_action_instruction,
                                &account_infos,
                                &[auth_seeds_vec.as_slice()],
                            )?;
                        }
                        (true, true) => {
                            anchor_lang::solana_program::program::invoke_signed(
                                &mint_action_instruction,
                                &account_infos,
                                &[],
                            )?;
                        }
                    }
                }
            }
        } else {
            quote! {}
        };

        // Build the full compressed finalize block
        quote! {
            // Compressed accounts finalization
            {
                // Deserialize instruction args to access compression params
                // This is the same pattern as try_accounts uses
                #ix_de
                
                // Now we can access compression_params and other instruction args by name!

                // Build CPI accounts with CPI context signer (no clone)
                let cpi_accounts = light_sdk::cpi::CpiAccountsSmall::new_with_config(
                    &self.#fee_payer,
                    _remaining,
                    light_sdk_types::CpiAccountsConfig::new_with_cpi_context(crate::LIGHT_CPI_SIGNER),
                );

                // Load compression config
                let compression_config_data = light_sdk::compressible::CompressibleConfig::load_checked(&self.#compression_config, &crate::ID)?;
                let address_space = compression_config_data.address_space;

                // Collect compressed infos for all compressible accounts (pre-allocate)
                let mut all_compressed_infos = Vec::with_capacity(#compressible_count as usize);
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
    
    // Generate auto-close for CPDA fields with compress_on_init
    let compress_on_init_fields: Vec<_> = accs.fields.iter().filter_map(|af| {
        if let AccountField::Field(f) = af {
            if let Some(cpda) = &f.constraints.cpda {
                if cpda.compress_on_init {
                    return Some(&f.ident);
                }
            }
        }
        None
    }).collect();
    
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
                    Ty::Account(_) if f.constraints.cpda.is_some() => {
                        // Skip CPDA accounts - handled in batched finalize
                        quote! {}
                    }
                    _ => quote! {
                        anchor_lang::AccountsFinalize::finalize(&self.#ident, program_id, _remaining, _ix_data, _bumps)
                            .map_err(|e| e.with_account_name(#name_str))?;
                    },
                }
            }
        })
        .collect();
    quote! {
        #[automatically_derived]
        impl<#combined_generics> anchor_lang::AccountsFinalize<#trait_generics, #bumps_struct_name> for #name<#struct_generics> #where_clause{
            fn finalize(
                &self,
                program_id: &anchor_lang::solana_program::pubkey::Pubkey,
                _remaining: &[anchor_lang::solana_program::account_info::AccountInfo<#trait_generics>],
                _ix_data: &[u8],
                _bumps: &#bumps_struct_name,
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
