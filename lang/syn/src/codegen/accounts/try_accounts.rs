use crate::codegen::accounts::{bumps, constraints, generics, ParsedGenerics};
use crate::{AccountField, AccountsStruct};
use quote::quote;
use syn::Expr;

// Generates the `Accounts` trait implementation.
pub fn generate(accs: &AccountsStruct) -> proc_macro2::TokenStream {
    let name = &accs.ident;
    let ParsedGenerics {
        combined_generics,
        trait_generics,
        struct_generics,
        where_clause,
    } = generics(accs);

    // Deserialization for each field
    let all_fields = &accs.fields;  // Capture all fields for CMint seed extraction
    let deser_fields: Vec<proc_macro2::TokenStream> = accs
        .fields
        .iter()
        .map(|af: &AccountField| {
            match af {
                AccountField::CompositeField(s) => {
                    let name = &s.ident;
                    let ty = &s.raw_field.ty;
                    quote! {
                        #[cfg(feature = "anchor-debug")]
                        ::solana_program::log::sol_log(stringify!(#name));
                        let #name: #ty = anchor_lang::Accounts::try_accounts(__program_id, __accounts, __ix_data, &mut __bumps.#name, __reallocs)?;
                    }
                }
                AccountField::Field(f) => {
                    // `init` and `zero` accounts are special cased as they are
                    // deserialized by constraints. Here, we just take out the
                    // AccountInfo for later use at constraint validation time.
                    if is_init(af) || f.constraints.zeroed.is_some()  {
                        let name = &f.ident;
                        // Optional accounts have slightly different behavior here and
                        // we can't leverage the try_accounts implementation for zero and init.
                        if f.is_optional {
                            // Thus, this block essentially reimplements the try_accounts 
                            // behavior with optional accounts minus the deserialziation.
                            let empty_behavior = if cfg!(feature = "allow-missing-optionals") {
                                quote!{ None }
                            } else {
                                quote!{ return Err(anchor_lang::error::ErrorCode::AccountNotEnoughKeys.into()); }
                            };
                            quote! {
                                let #name = if __accounts.is_empty() {
                                    #empty_behavior
                                } else if __accounts[0].key == __program_id {
                                    *__accounts = &__accounts[1..];
                                    None
                                } else {
                                    let account = &__accounts[0];
                                    *__accounts = &__accounts[1..];
                                    Some(account)
                                };
                            }
                        } else {
                            quote!{
                                if __accounts.is_empty() {
                                    return Err(anchor_lang::error::ErrorCode::AccountNotEnoughKeys.into());
                                }
                                let #name = &__accounts[0];
                                *__accounts = &__accounts[1..];
                            }
                        }
                    } else {
                        let name = f.ident.to_string();
                        let typed_name = f.typed_ident();
                        
                        // Special handling for CMint fields to populate constraints
                        if matches!(&f.ty, crate::Ty::CMint(_)) {
                            let ident = &f.ident;
                            let cmint_setup = if let Some(cmint_group) = &f.constraints.cmint {
                                let authority = cmint_group.authority.as_ref().map(|a| quote! { {
                                    let __auth = &#a;
                                    #ident.authority = Some(__auth.key());
                                }});
                                let decimals = cmint_group.decimals.map(|d| quote! { 
                                    #ident.decimals = Some(#d);
                                });
                                let mint_signer = cmint_group.mint_signer.as_ref().map(|s| quote! { {
                                    let __signer = &#s;
                                    #ident.mint_signer = Some(__signer.to_account_info());
                                }});
                                
                                // Auto-extract seeds and bump from linked mint_signer account if specified
                                let (mint_signer_seeds, mint_signer_bump) = if let Some(signer_ref) = &cmint_group.mint_signer {
                                    // Try to find the linked account's seeds and bump from its constraints
                                    // Look for the account with matching name in the fields
                                    let mut found_seeds = None;
                                    let mut found_bump = None;
                                    
                                    // Extract the field name from the expression
                                    let signer_name = quote! { #signer_ref }.to_string();
                                    
                                    // Find the corresponding field and extract its seeds
                                    for field in all_fields {
                                        if let crate::AccountField::Field(other_f) = field {
                                            if other_f.ident.to_string() == signer_name {
                                                if let Some(seeds_group) = &other_f.constraints.seeds {
                                                    let seeds = &seeds_group.seeds;
                                                    let seed_exprs: Vec<_> = seeds.iter().map(|s| {
                                                        quote! { Vec::from(#s.as_ref()) }
                                                    }).collect();
                                                    found_seeds = Some(quote! {
                                                        #ident.mint_signer_seeds = Some(vec![#(#seed_exprs),*]);
                                                    });
                                                    
                                                    if let Some(bump) = &seeds_group.bump {
                                                        found_bump = Some(quote! {
                                                            #ident.mint_signer_bump = Some(#bump);
                                                        });
                                                    }
                                                }
                                                break;
                                            }
                                        }
                                    }
                                    
                                    // If we found seeds from the linked account, use those
                                    // Otherwise fall back to explicitly specified seeds
                                    if found_seeds.is_some() {
                                        (found_seeds, found_bump)
                                    } else {
                                        // Fall back to explicitly provided seeds if any
                                        let seeds = cmint_group.mint_signer_seeds.as_ref().map(|seeds| {
                                            let seed_exprs: Vec<_> = seeds.iter().map(|s| {
                                                quote! { Vec::from(#s.as_ref()) }
                                            }).collect();
                                            quote! {
                                                #ident.mint_signer_seeds = Some(vec![#(#seed_exprs),*]);
                                            }
                                        });
                                        let bump = cmint_group.mint_signer_bump.as_ref().map(|b| quote! {
                                            #ident.mint_signer_bump = Some(#b);
                                        });
                                        (seeds, bump)
                                    }
                                } else {
                                    // No mint_signer specified, use explicitly provided seeds
                                    let seeds = cmint_group.mint_signer_seeds.as_ref().map(|seeds| {
                                        let seed_exprs: Vec<_> = seeds.iter().map(|s| {
                                            quote! { Vec::from(#s.as_ref()) }
                                        }).collect();
                                        quote! {
                                            #ident.mint_signer_seeds = Some(vec![#(#seed_exprs),*]);
                                        }
                                    });
                                    let bump = cmint_group.mint_signer_bump.as_ref().map(|b| quote! {
                                        #ident.mint_signer_bump = Some(#b);
                                    });
                                    (seeds, bump)
                                };
                                let program_authority_seeds = cmint_group.program_authority_seeds.as_ref().map(|seeds| {
                                    let seed_exprs: Vec<_> = seeds.iter().map(|s| {
                                        quote! { Vec::from(#s.as_ref()) }
                                    }).collect();
                                    quote! {
                                        #ident.program_authority_seeds = Some(vec![#(#seed_exprs),*]);
                                    }
                                });
                                let program_authority_bump = cmint_group.program_authority_bump.as_ref().map(|b| quote! {
                                    #ident.program_authority_bump = Some(#b);
                                });
                                
                                quote! {
                                    #authority
                                    #decimals
                                    #mint_signer
                                    #mint_signer_seeds
                                    #mint_signer_bump
                                    #program_authority_seeds
                                    #program_authority_bump
                                }
                            } else {
                                quote! {}
                            };
                            
                            quote! {
                                #[cfg(feature = "anchor-debug")]
                                ::solana_program::log::sol_log(stringify!(#typed_name));
                                let mut #typed_name = anchor_lang::Accounts::try_accounts(__program_id, __accounts, __ix_data, __bumps, __reallocs)
                                    .map_err(|e| e.with_account_name(#name))?;
                                #cmint_setup
                            }
                        } else {
                            // Check if this is an InterfaceAccount<Mint> with mint::token_program constraint
                            // IMPORTANT: Only mint accounts can be CToken
                            // Token accounts must always be decompressed on-chain, even for CToken
                            let token_program_constraint = if matches!(&f.ty, crate::Ty::InterfaceAccount(_)) {
                                // ONLY check mint:: constraints, NOT token:: constraints
                                f.constraints.mint.as_ref()
                                    .and_then(|mc| mc.token_program.as_ref())
                            } else {
                                None
                            };
                            
                            if let Some(token_prog_ref) = token_program_constraint {
                                // We have a token_program constraint - need to find its position in accounts
                                // to peek at the right account
                                let token_prog_name = quote! { #token_prog_ref }.to_string();
                                
                                // Find the index of the token_program field
                                let token_prog_index = accs.fields.iter().position(|field| {
                                    if let AccountField::Field(f) = field {
                                        f.ident.to_string() == token_prog_name
                                    } else {
                                        false
                                    }
                                });
                                
                                // Check if we need to wrap in Box
                                let is_boxed = if let crate::Ty::InterfaceAccount(iac) = &f.ty {
                                    iac.boxed
                                } else {
                                    false
                                };
                                
                                let interface_creation = if let Some(prog_idx) = token_prog_index {
                                    // Calculate relative position of token_program
                                    let current_idx = accs.fields.iter().position(|field| {
                                        if let AccountField::Field(other_f) = field {
                                            std::ptr::eq(other_f, f)
                                        } else {
                                            false
                                        }
                                    }).unwrap_or(0);
                                    
                                    if prog_idx < current_idx {
                                        // Backward lookup: token_program was already deserialized
                                        quote! {
                                            if __accounts.is_empty() {
                                                return Err(anchor_lang::error::ErrorCode::AccountNotEnoughKeys.into());
                                            }
                                            let __acc = &__accounts[0];
                                            *__accounts = &__accounts[1..];
                                            
                                            // Use already-deserialized token_program to check if CToken
                                            let __is_ctoken = #token_prog_ref.key() == &anchor_lang::CTOKEN_ID;
                                            
                                            if __is_ctoken {
                                                // Use try_from_ctoken to skip deserialization for compressed accounts
                                                anchor_lang::accounts::interface_account::InterfaceAccount::try_from_ctoken(__acc)
                                                    .map_err(|e| e.with_account_name(#name))?
                                            } else {
                                                // Normal deserialization for SPL/T22
                                                anchor_lang::accounts::interface_account::InterfaceAccount::try_from(__acc)
                                                    .map_err(|e| e.with_account_name(#name))?
                                            }
                                        }
                                    } else {
                                        // Forward lookup: peek ahead to token_program
                                        let peek_offset = prog_idx - current_idx;
                                        
                                        quote! {
                                            if __accounts.is_empty() {
                                                return Err(anchor_lang::error::ErrorCode::AccountNotEnoughKeys.into());
                                            }
                                            let __acc = &__accounts[0];
                                            
                                            // Peek ahead to check if token_program is CTOKEN_ID
                                            let __is_ctoken = if __accounts.len() > #peek_offset {
                                                __accounts[#peek_offset].key == &anchor_lang::CTOKEN_ID
                                            } else {
                                                false
                                            };
                                            
                                            *__accounts = &__accounts[1..];
                                            
                                            if __is_ctoken {
                                                // Use try_from_ctoken to skip deserialization for compressed accounts
                                                anchor_lang::accounts::interface_account::InterfaceAccount::try_from_ctoken(__acc)
                                                    .map_err(|e| e.with_account_name(#name))?
                                            } else {
                                                // Normal deserialization for SPL/T22
                                                anchor_lang::accounts::interface_account::InterfaceAccount::try_from(__acc)
                                                    .map_err(|e| e.with_account_name(#name))?
                                            }
                                        }
                                    }
                                } else {
                                    // Check if this is a state field reference (e.g., pool_state.token_0_program)
                                    if token_prog_name.contains('.') {
                                        // Runtime check - read token_program from already-deserialized account
                                        quote! {
                                            if __accounts.is_empty() {
                                                return Err(anchor_lang::error::ErrorCode::AccountNotEnoughKeys.into());
                                            }
                                            let __acc = &__accounts[0];
                                            *__accounts = &__accounts[1..];
                                            
                                            // Use runtime token_program value from state field
                                            anchor_lang::accounts::interface_account::InterfaceAccount::try_from_with_token_program(__acc, &#token_prog_ref)
                                                .map_err(|e| e.with_account_name(#name))?
                                        }
                                    } else {
                                        // Fallback if we can't find the token_program field in accounts
                                        quote! {
                                            if __accounts.is_empty() {
                                                return Err(anchor_lang::error::ErrorCode::AccountNotEnoughKeys.into());
                                            }
                                            let __acc = &__accounts[0];
                                            *__accounts = &__accounts[1..];
                                            
                                            // Default to normal deserialization if we can't determine token_program
                                            anchor_lang::accounts::interface_account::InterfaceAccount::try_from(__acc)
                                                .map_err(|e| e.with_account_name(#name))?
                                        }
                                    }
                                };
                                
                                if is_boxed {
                                    quote! {
                                        #[cfg(feature = "anchor-debug")]
                                        ::solana_program::log::sol_log(stringify!(#typed_name));
                                        let #typed_name = Box::new({
                                            #interface_creation
                                        });
                                    }
                                } else {
                                    quote! {
                                        #[cfg(feature = "anchor-debug")]
                                        ::solana_program::log::sol_log(stringify!(#typed_name));
                                        let #typed_name = {
                                            #interface_creation
                                        };
                                    }
                                }
                            } else {
                                quote! {
                                    #[cfg(feature = "anchor-debug")]
                                    ::solana_program::log::sol_log(stringify!(#typed_name));
                                    let #typed_name = anchor_lang::Accounts::try_accounts(__program_id, __accounts, __ix_data, __bumps, __reallocs)
                                        .map_err(|e| e.with_account_name(#name))?;
                                }
                            }
                        }
                    }
                }
            }
        })
        .collect();

    let constraints = generate_constraints(accs);
    let accounts_instance = generate_accounts_instance(accs);
    let bumps_struct_name = bumps::generate_bumps_name(&accs.ident);

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
                let mut __ix_data = __ix_data;
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

    quote! {
        #[automatically_derived]
        impl<#combined_generics> anchor_lang::Accounts<#trait_generics, #bumps_struct_name> for #name<#struct_generics> #where_clause {
            #[inline(never)]
            fn try_accounts(
                __program_id: &anchor_lang::solana_program::pubkey::Pubkey,
                __accounts: &mut &#trait_generics [anchor_lang::solana_program::account_info::AccountInfo<#trait_generics>],
                __ix_data: &[u8],
                __bumps: &mut #bumps_struct_name,
                __reallocs: &mut std::collections::BTreeSet<anchor_lang::solana_program::pubkey::Pubkey>,
            ) -> anchor_lang::Result<Self> {
                // Deserialize instruction, if declared.
                #ix_de
                // Deserialize each account.
                #(#deser_fields)*
                // Execute accounts constraints.
                #constraints
                // Success. Return the validated accounts.
                Ok(#accounts_instance)
            }
        }
    }
}

pub fn generate_constraints(accs: &AccountsStruct) -> proc_macro2::TokenStream {
    let non_init_fields: Vec<&AccountField> =
        accs.fields.iter().filter(|af| !is_init(af)).collect();

    // Deserialization for each pda init field. This must be after
    // the initial extraction from the accounts slice and before access_checks.
    let init_fields: Vec<proc_macro2::TokenStream> = accs
        .fields
        .iter()
        .filter_map(|af| match af {
            AccountField::CompositeField(_s) => None,
            AccountField::Field(f) => match is_init(af) {
                false => None,
                true => Some(f),
            },
        })
        .map(|f| constraints::generate(f, accs))
        .collect();

    // Constraint checks for each account fields.
    let access_checks: Vec<proc_macro2::TokenStream> = non_init_fields
        .iter()
        .map(|af: &&AccountField| match af {
            AccountField::Field(f) => constraints::generate(f, accs),
            AccountField::CompositeField(s) => constraints::generate_composite(s),
        })
        .collect();

    quote! {
        #(#init_fields)*
        #(#access_checks)*
    }
}

pub fn generate_accounts_instance(accs: &AccountsStruct) -> proc_macro2::TokenStream {
    let name = &accs.ident;
    // Each field in the final deserialized accounts struct.
    let return_tys: Vec<proc_macro2::TokenStream> = accs
        .fields
        .iter()
        .map(|f: &AccountField| {
            let name = match f {
                AccountField::CompositeField(s) => &s.ident,
                AccountField::Field(f) => &f.ident,
            };
            quote! {
                #name
            }
        })
        .collect();

    quote! {
        #name {
            #(#return_tys),*
        }
    }
}

fn is_init(af: &AccountField) -> bool {
    match af {
        AccountField::CompositeField(_s) => false,
        AccountField::Field(f) => f.constraints.init.is_some(),
    }
}
