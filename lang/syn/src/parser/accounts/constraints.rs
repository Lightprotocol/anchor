use crate::*;
use syn::parse::{Error as ParseError, Result as ParseResult};
use syn::{bracketed, Token};

pub fn parse(f: &syn::Field, f_ty: Option<&Ty>) -> ParseResult<ConstraintGroup> {
    parse_with_instruction(f, f_ty, &None)
}

pub fn parse_with_instruction(
    f: &syn::Field,
    f_ty: Option<&Ty>,
    instruction_api: &Option<Punctuated<Expr, Comma>>,
) -> ParseResult<ConstraintGroup> {
    let mut constraints = ConstraintGroupBuilder::new(f_ty);
    constraints.set_instruction_api(instruction_api.clone());
    for attr in f.attrs.iter().filter(is_account) {
        for c in attr.parse_args_with(Punctuated::<ConstraintToken, Comma>::parse_terminated)? {
            constraints.add(c)?;
        }
    }
    let account_constraints = constraints.build()?;

    Ok(account_constraints)
}

pub fn is_account(attr: &&syn::Attribute) -> bool {
    attr.path
        .get_ident()
        .is_some_and(|ident| ident == "account")
}

// Parses a single constraint from a parse stream for `#[account(<STREAM>)]`.
pub fn parse_token(stream: ParseStream) -> ParseResult<ConstraintToken> {
    let ident = stream.call(Ident::parse_any)?;
    let kw = ident.to_string();

    let c = match kw.as_str() {
        "init" => ConstraintToken::Init(Context::new(
            ident.span(),
            ConstraintInit { if_needed: false },
        )),
        "init_if_needed" => ConstraintToken::Init(Context::new(
            ident.span(),
            ConstraintInit { if_needed: true },
        )),
        "zero" => ConstraintToken::Zeroed(Context::new(ident.span(), ConstraintZeroed {})),
        "mut" => ConstraintToken::Mut(Context::new(
            ident.span(),
            ConstraintMut {
                error: parse_optional_custom_error(&stream)?,
            },
        )),
        "signer" => ConstraintToken::Signer(Context::new(
            ident.span(),
            ConstraintSigner {
                error: parse_optional_custom_error(&stream)?,
            },
        )),
        "executable" => {
            ConstraintToken::Executable(Context::new(ident.span(), ConstraintExecutable {}))
        }
        "mint" => {
            stream.parse::<Token![:]>()?;
            stream.parse::<Token![:]>()?;
            let kw = stream.call(Ident::parse_any)?.to_string();
            stream.parse::<Token![=]>()?;

            let span = ident
                .span()
                .join(stream.span())
                .unwrap_or_else(|| ident.span());

            match kw.as_str() {
                // Shared constraint names - return both SPL and CToken variants
                // The validator will use the correct one based on account type
                "authority" => {
                    // For CMint accounts, this creates a CTokenMintAuthority
                    // For Mint accounts, validator will convert to MintAuthority
                    ConstraintToken::CTokenMintAuthority(Context::new(
                        span,
                        ConstraintCTokenMintAuthority {
                            authority: stream.parse()?,
                        },
                    ))
                },
                "freeze_authority" => {
                    ConstraintToken::CTokenMintFreezeAuthority(Context::new(
                        span,
                        ConstraintCTokenMintFreezeAuthority {
                            freeze_authority: stream.parse()?,
                        },
                    ))
                },
                "decimals" => {
                    // Try parsing as u8 first (CToken style), fall back to Expr (SPL style)
                    match stream.parse::<syn::LitInt>() {
                        Ok(lit) => ConstraintToken::CTokenMintDecimals(Context::new(
                            span,
                            ConstraintCTokenMintDecimals {
                                decimals: lit.base10_parse()?,
                            },
                        )),
                        Err(_) => {
                            // Reset stream and try as Expr for SPL
                            ConstraintToken::MintDecimals(Context::new(
                                span,
                                ConstraintMintDecimals {
                                    decimals: stream.parse()?,
                                },
                            ))
                        }
                    }
                },
                "token_program" => ConstraintToken::MintTokenProgram(Context::new(
                    span,
                    ConstraintTokenProgram {
                        token_program: stream.parse()?,
                    },
                )),
                "compressed" => {
                    let lit_bool: syn::LitBool = stream.parse()?;
                    ConstraintToken::MintCompressed(Context::new(
                        span,
                        ConstraintMintCompressed {
                            compressed: lit_bool.value,
                        },
                    ))
                }
                // CToken Mint constraints (for CMint accounts)
                "payer" => ConstraintToken::CTokenMintPayer(Context::new(
                    span,
                    ConstraintCTokenMintPayer {
                        payer: stream.parse()?,
                    },
                )),
                "mint_signer" => ConstraintToken::CTokenMintSigner(Context::new(
                    span,
                    ConstraintCTokenMintSigner {
                        signer: stream.parse()?,
                    },
                )),
                "mint_signer_seeds" => {
                    let seeds_stream;
                    let bracket = bracketed!(seeds_stream in stream);
                    let seeds: Punctuated<Expr, Token![,]> = seeds_stream.parse_terminated(Expr::parse)?;
                    ConstraintToken::CTokenMintSignerSeeds(Context::new(
                        span.join(bracket.span).unwrap_or(span),
                        ConstraintCTokenMintSignerSeeds { seeds: seeds.into_iter().collect() },
                    ))
                }
                "mint_signer_bump" => ConstraintToken::CTokenMintSignerBump(Context::new(
                    span,
                    ConstraintCTokenMintSignerBump {
                        bump: stream.parse()?,
                    },
                )),
                "program_authority_seeds" => {
                    let seeds_stream;
                    let bracket = bracketed!(seeds_stream in stream);
                    let seeds: Punctuated<Expr, Token![,]> = seeds_stream.parse_terminated(Expr::parse)?;
                    ConstraintToken::CTokenMintProgramAuthoritySeeds(Context::new(
                        span.join(bracket.span).unwrap_or(span),
                        ConstraintCTokenMintProgramAuthoritySeeds { seeds: seeds.into_iter().collect() },
                    ))
                }
                "program_authority_bump" => ConstraintToken::CTokenMintProgramAuthorityBump(Context::new(
                    span,
                    ConstraintCTokenMintProgramAuthorityBump {
                        bump: stream.parse()?,
                    },
                )),
                "address_tree_info" => ConstraintToken::CTokenMintAddressTreeInfo(Context::new(
                    span,
                    ConstraintCTokenMintAddressTreeInfo {
                        address_tree_info: stream.parse()?,
                    },
                )),
                "proof" => ConstraintToken::CTokenMintProof(Context::new(
                    span,
                    ConstraintCTokenMintProof {
                        proof: stream.parse()?,
                    },
                )),
                "output_state_tree_index" => ConstraintToken::CTokenMintOutputStateTreeIndex(Context::new(
                    span,
                    ConstraintCTokenMintOutputStateTreeIndex {
                        output_state_tree_index: stream.parse()?,
                    },
                )),
                _ => return Err(ParseError::new(ident.span(), "Invalid mint attribute")),
            }
        }
        "extensions" => {
            stream.parse::<Token![:]>()?;
            stream.parse::<Token![:]>()?;
            let kw = stream.call(Ident::parse_any)?.to_string();

            match kw.as_str() {
                "group_pointer" => {
                    stream.parse::<Token![:]>()?;
                    stream.parse::<Token![:]>()?;
                    let kw = stream.call(Ident::parse_any)?.to_string();
                    stream.parse::<Token![=]>()?;

                    let span = ident
                        .span()
                        .join(stream.span())
                        .unwrap_or_else(|| ident.span());

                    match kw.as_str() {
                        "authority" => {
                            ConstraintToken::ExtensionGroupPointerAuthority(Context::new(
                                span,
                                ConstraintExtensionAuthority {
                                    authority: stream.parse()?,
                                },
                            ))
                        }
                        "group_address" => {
                            ConstraintToken::ExtensionGroupPointerGroupAddress(Context::new(
                                span,
                                ConstraintExtensionGroupPointerGroupAddress {
                                    group_address: stream.parse()?,
                                },
                            ))
                        }
                        _ => return Err(ParseError::new(ident.span(), "Invalid attribute")),
                    }
                }
                "group_member_pointer" => {
                    stream.parse::<Token![:]>()?;
                    stream.parse::<Token![:]>()?;
                    let kw = stream.call(Ident::parse_any)?.to_string();
                    stream.parse::<Token![=]>()?;

                    let span = ident
                        .span()
                        .join(stream.span())
                        .unwrap_or_else(|| ident.span());

                    match kw.as_str() {
                        "authority" => {
                            ConstraintToken::ExtensionGroupMemberPointerAuthority(Context::new(
                                span,
                                ConstraintExtensionAuthority {
                                    authority: stream.parse()?,
                                },
                            ))
                        }
                        "member_address" => {
                            ConstraintToken::ExtensionGroupMemberPointerMemberAddress(Context::new(
                                span,
                                ConstraintExtensionGroupMemberPointerMemberAddress {
                                    member_address: stream.parse()?,
                                },
                            ))
                        }
                        _ => return Err(ParseError::new(ident.span(), "Invalid attribute")),
                    }
                }
                "metadata_pointer" => {
                    stream.parse::<Token![:]>()?;
                    stream.parse::<Token![:]>()?;
                    let kw = stream.call(Ident::parse_any)?.to_string();
                    stream.parse::<Token![=]>()?;

                    let span = ident
                        .span()
                        .join(stream.span())
                        .unwrap_or_else(|| ident.span());

                    match kw.as_str() {
                        "authority" => {
                            ConstraintToken::ExtensionMetadataPointerAuthority(Context::new(
                                span,
                                ConstraintExtensionAuthority {
                                    authority: stream.parse()?,
                                },
                            ))
                        }
                        "metadata_address" => {
                            ConstraintToken::ExtensionMetadataPointerMetadataAddress(Context::new(
                                span,
                                ConstraintExtensionMetadataPointerMetadataAddress {
                                    metadata_address: stream.parse()?,
                                },
                            ))
                        }
                        _ => return Err(ParseError::new(ident.span(), "Invalid attribute")),
                    }
                }
                "close_authority" => {
                    stream.parse::<Token![:]>()?;
                    stream.parse::<Token![:]>()?;
                    let kw = stream.call(Ident::parse_any)?.to_string();
                    stream.parse::<Token![=]>()?;

                    let span = ident
                        .span()
                        .join(stream.span())
                        .unwrap_or_else(|| ident.span());

                    match kw.as_str() {
                        "authority" => ConstraintToken::ExtensionCloseAuthority(Context::new(
                            span,
                            ConstraintExtensionAuthority {
                                authority: stream.parse()?,
                            },
                        )),
                        _ => return Err(ParseError::new(ident.span(), "Invalid attribute")),
                    }
                }
                "permanent_delegate" => {
                    stream.parse::<Token![:]>()?;
                    stream.parse::<Token![:]>()?;
                    let kw = stream.call(Ident::parse_any)?.to_string();
                    stream.parse::<Token![=]>()?;

                    let span = ident
                        .span()
                        .join(stream.span())
                        .unwrap_or_else(|| ident.span());

                    match kw.as_str() {
                        "delegate" => ConstraintToken::ExtensionPermanentDelegate(Context::new(
                            span,
                            ConstraintExtensionPermanentDelegate {
                                permanent_delegate: stream.parse()?,
                            },
                        )),
                        _ => return Err(ParseError::new(ident.span(), "Invalid attribute")),
                    }
                }
                "transfer_hook" => {
                    stream.parse::<Token![:]>()?;
                    stream.parse::<Token![:]>()?;
                    let kw = stream.call(Ident::parse_any)?.to_string();
                    stream.parse::<Token![=]>()?;

                    let span = ident
                        .span()
                        .join(stream.span())
                        .unwrap_or_else(|| ident.span());

                    match kw.as_str() {
                        "authority" => ConstraintToken::ExtensionTokenHookAuthority(Context::new(
                            span,
                            ConstraintExtensionAuthority {
                                authority: stream.parse()?,
                            },
                        )),
                        "program_id" => ConstraintToken::ExtensionTokenHookProgramId(Context::new(
                            span,
                            ConstraintExtensionTokenHookProgramId {
                                program_id: stream.parse()?,
                            },
                        )),
                        _ => return Err(ParseError::new(ident.span(), "Invalid attribute")),
                    }
                }
                _ => return Err(ParseError::new(ident.span(), "Invalid attribute")),
            }
        }
        "token" => {
            stream.parse::<Token![:]>()?;
            stream.parse::<Token![:]>()?;
            let kw = stream.call(Ident::parse_any)?.to_string();
            stream.parse::<Token![=]>()?;

            let span = ident
                .span()
                .join(stream.span())
                .unwrap_or_else(|| ident.span());

            match kw.as_str() {
                "mint" => ConstraintToken::TokenMint(Context::new(
                    span,
                    ConstraintTokenMint {
                        mint: stream.parse()?,
                    },
                )),
                "authority" => ConstraintToken::TokenAuthority(Context::new(
                    span,
                    ConstraintTokenAuthority {
                        auth: stream.parse()?,
                    },
                )),
                "token_program" => ConstraintToken::TokenTokenProgram(Context::new(
                    span,
                    ConstraintTokenProgram {
                        token_program: stream.parse()?,
                    },
                )),
                _ => return Err(ParseError::new(ident.span(), "Invalid attribute")),
            }
        }
            "cpda" => {
                stream.parse::<Token![::]>()?;
                let kw = stream.call(Ident::parse_any)?.to_string();
                stream.parse::<Token![=]>()?;

                let span = ident
                    .span()
                    .join(stream.span())
                    .unwrap_or_else(|| ident.span());

                match kw.as_str() {
                    "authority" => ConstraintToken::CPDAAuthority(Context::new(
                        span,
                        ConstraintCPDAAuthority {
                            authority: stream.parse()?,
                        },
                    )),
                    "address_tree_info" => ConstraintToken::CPDAAddressTreeInfo(Context::new(
                        span,
                        ConstraintCPDAAddressTreeInfo {
                            address_tree_info: stream.parse()?,
                        },
                    )),
                    "proof" => ConstraintToken::CPDAProof(Context::new(
                        span,
                        ConstraintCPDAProof {
                            proof: stream.parse()?,
                        },
                    )),
                    "output_state_tree_index" => ConstraintToken::CPDAOutputStateTreeIndex(Context::new(
                        span,
                        ConstraintCPDAOutputStateTreeIndex {
                            output_state_tree_index: stream.parse()?,
                        },
                    )),
                    _ => {
                        stream.parse::<Token![=]>()?;
                        return Err(ParseError::new(
                            ident.span(),
                            format!("invalid cpda constraint: {}", kw),
                        ));
                    }
                }
            }
            "compress_on_init" => {
                // This is a special flag that marks a CPDA for immediate compression
                ConstraintToken::CPDACompressOnInit(Context::new(
                    ident.span(),
                    ConstraintCPDACompressOnInit {},
                ))
            }
            "cctoken" => {
                // Marks this account as a compressible compressed-token account
                ConstraintToken::CCToken(Context::new(
                    ident.span(),
                    ConstraintCCToken {
                        mint: None,
                    },
                ))
            }
            "metadata" => {
            stream.parse::<Token![:]>()?;
            stream.parse::<Token![:]>()?;
            let kw = stream.call(Ident::parse_any)?.to_string();
            stream.parse::<Token![=]>()?;

            let span = ident
                .span()
                .join(stream.span())
                .unwrap_or_else(|| ident.span());

            match kw.as_str() {
                        "name" => {
                            let name_expr: Expr = stream.parse()?;
                            validate_metadata_size_limit(&name_expr, "name", 32, span)?;
                            ConstraintToken::MetadataName(Context::new(
                                span,
                                ConstraintMetadataName {
                                    name: name_expr,
                                },
                            ))
                        },
                        "symbol" => {
                            let symbol_expr: Expr = stream.parse()?;
                            validate_metadata_size_limit(&symbol_expr, "symbol", 10, span)?;
                            ConstraintToken::MetadataSymbol(Context::new(
                                span,
                                ConstraintMetadataSymbol {
                                    symbol: symbol_expr,
                                },
                            ))
                        },
                        "uri" => {
                            let uri_expr: Expr = stream.parse()?;
                            validate_metadata_size_limit(&uri_expr, "uri", 200, span)?;
                            ConstraintToken::MetadataUri(Context::new(
                                span,
                                ConstraintMetadataUri {
                                    uri: uri_expr,
                                },
                            ))
                        },
                        "update_authority" => ConstraintToken::MetadataUpdateAuthority(Context::new(
                            span,
                            ConstraintMetadataUpdateAuthority {
                                update_authority: stream.parse()?,
                            },
                        )),
                        "additional" => ConstraintToken::MetadataAdditional(Context::new(
                            span,
                            ConstraintMetadataAdditional {
                                additional: stream.parse()?,
                            },
                        )),
                _ => return Err(ParseError::new(ident.span(), "Invalid metadata attribute")),
            }
        }
        "associated_token" => {
            stream.parse::<Token![:]>()?;
            stream.parse::<Token![:]>()?;
            let kw = stream.call(Ident::parse_any)?.to_string();
            stream.parse::<Token![=]>()?;

            let span = ident
                .span()
                .join(stream.span())
                .unwrap_or_else(|| ident.span());

            match kw.as_str() {
                "mint" => ConstraintToken::AssociatedTokenMint(Context::new(
                    span,
                    ConstraintTokenMint {
                        mint: stream.parse()?,
                    },
                )),
                "authority" => ConstraintToken::AssociatedTokenAuthority(Context::new(
                    span,
                    ConstraintTokenAuthority {
                        auth: stream.parse()?,
                    },
                )),
                "token_program" => ConstraintToken::AssociatedTokenTokenProgram(Context::new(
                    span,
                    ConstraintTokenProgram {
                        token_program: stream.parse()?,
                    },
                )),
                _ => return Err(ParseError::new(ident.span(), "Invalid attribute")),
            }
        }
        "bump" => {
            let bump = {
                if stream.peek(Token![=]) {
                    stream.parse::<Token![=]>()?;
                    Some(stream.parse()?)
                } else {
                    None
                }
            };
            ConstraintToken::Bump(Context::new(ident.span(), ConstraintTokenBump { bump }))
        }
        "seeds" => {
            if stream.peek(Token![:]) {
                stream.parse::<Token![:]>()?;
                stream.parse::<Token![:]>()?;
                let kw = stream.call(Ident::parse_any)?.to_string();
                stream.parse::<Token![=]>()?;

                let span = ident
                    .span()
                    .join(stream.span())
                    .unwrap_or_else(|| ident.span());

                match kw.as_str() {
                    "program" => ConstraintToken::ProgramSeed(Context::new(
                        span,
                        ConstraintProgramSeed {
                            program_seed: stream.parse()?,
                        },
                    )),
                    _ => return Err(ParseError::new(ident.span(), "Invalid attribute")),
                }
            } else {
                stream.parse::<Token![=]>()?;
                let span = ident
                    .span()
                    .join(stream.span())
                    .unwrap_or_else(|| ident.span());
                let seeds;
                let bracket = bracketed!(seeds in stream);
                ConstraintToken::Seeds(Context::new(
                    span.join(bracket.span).unwrap_or(span),
                    ConstraintSeeds {
                        seeds: seeds.parse_terminated(Expr::parse)?,
                    },
                ))
            }
        }
        "realloc" => {
            if stream.peek(Token![=]) {
                stream.parse::<Token![=]>()?;
                let span = ident
                    .span()
                    .join(stream.span())
                    .unwrap_or_else(|| ident.span());
                ConstraintToken::Realloc(Context::new(
                    span,
                    ConstraintRealloc {
                        space: stream.parse()?,
                    },
                ))
            } else {
                stream.parse::<Token![:]>()?;
                stream.parse::<Token![:]>()?;
                let kw = stream.call(Ident::parse_any)?.to_string();
                stream.parse::<Token![=]>()?;

                let span = ident
                    .span()
                    .join(stream.span())
                    .unwrap_or_else(|| ident.span());

                match kw.as_str() {
                    "payer" => ConstraintToken::ReallocPayer(Context::new(
                        span,
                        ConstraintReallocPayer {
                            target: stream.parse()?,
                        },
                    )),
                    "zero" => ConstraintToken::ReallocZero(Context::new(
                        span,
                        ConstraintReallocZero {
                            zero: stream.parse()?,
                        },
                    )),
                    _ => return Err(ParseError::new(ident.span(), "Invalid attribute. realloc::payer and realloc::zero are the only valid attributes")),
                }
            }
        }
        _ => {
            stream.parse::<Token![=]>()?;
            let span = ident
                .span()
                .join(stream.span())
                .unwrap_or_else(|| ident.span());
            match kw.as_str() {
                "has_one" => ConstraintToken::HasOne(Context::new(
                    span,
                    ConstraintHasOne {
                        join_target: stream.parse()?,
                        error: parse_optional_custom_error(&stream)?,
                    },
                )),
                "owner" => ConstraintToken::Owner(Context::new(
                    span,
                    ConstraintOwner {
                        owner_address: stream.parse()?,
                        error: parse_optional_custom_error(&stream)?,
                    },
                )),
                "rent_exempt" => ConstraintToken::RentExempt(Context::new(
                    span,
                    match stream.parse::<Ident>()?.to_string().as_str() {
                        "skip" => ConstraintRentExempt::Skip,
                        "enforce" => ConstraintRentExempt::Enforce,
                        _ => {
                            return Err(ParseError::new(
                                span,
                                "rent_exempt must be either skip or enforce",
                            ))
                        }
                    },
                )),
                "payer" => ConstraintToken::Payer(Context::new(
                    span,
                    ConstraintPayer {
                        target: stream.parse()?,
                    },
                )),
                "space" => ConstraintToken::Space(Context::new(
                    span,
                    ConstraintSpace {
                        space: stream.parse()?,
                    },
                )),
                "constraint" => ConstraintToken::Raw(Context::new(
                    span,
                    ConstraintRaw {
                        raw: stream.parse()?,
                        error: parse_optional_custom_error(&stream)?,
                    },
                )),
                "close" => ConstraintToken::Close(Context::new(
                    span,
                    ConstraintClose {
                        sol_dest: stream.parse()?,
                    },
                )),
                "address" => ConstraintToken::Address(Context::new(
                    span,
                    ConstraintAddress {
                        address: stream.parse()?,
                        error: parse_optional_custom_error(&stream)?,
                    },
                )),
                _ => return Err(ParseError::new(ident.span(), "Invalid attribute")),
            }
        }
    };

    Ok(c)
}

fn parse_optional_custom_error(stream: &ParseStream) -> ParseResult<Option<Expr>> {
    if stream.peek(Token![@]) {
        stream.parse::<Token![@]>()?;
        stream.parse().map(Some)
    } else {
        Ok(None)
    }
}

#[derive(Default)]
pub struct ConstraintGroupBuilder<'ty> {
    pub f_ty: Option<&'ty Ty>,
    pub instruction_api: Option<Punctuated<Expr, Comma>>,
    pub init: Option<Context<ConstraintInit>>,
    pub zeroed: Option<Context<ConstraintZeroed>>,
    pub mutable: Option<Context<ConstraintMut>>,
    pub signer: Option<Context<ConstraintSigner>>,
    pub has_one: Vec<Context<ConstraintHasOne>>,
    pub raw: Vec<Context<ConstraintRaw>>,
    pub owner: Option<Context<ConstraintOwner>>,
    pub rent_exempt: Option<Context<ConstraintRentExempt>>,
    pub seeds: Option<Context<ConstraintSeeds>>,
    pub executable: Option<Context<ConstraintExecutable>>,
    pub payer: Option<Context<ConstraintPayer>>,
    pub space: Option<Context<ConstraintSpace>>,
    pub close: Option<Context<ConstraintClose>>,
    pub address: Option<Context<ConstraintAddress>>,
    pub token_mint: Option<Context<ConstraintTokenMint>>,
    pub token_authority: Option<Context<ConstraintTokenAuthority>>,
    pub token_token_program: Option<Context<ConstraintTokenProgram>>,
    pub associated_token_mint: Option<Context<ConstraintTokenMint>>,
    pub associated_token_authority: Option<Context<ConstraintTokenAuthority>>,
    pub associated_token_token_program: Option<Context<ConstraintTokenProgram>>,
    pub mint_authority: Option<Context<ConstraintMintAuthority>>,
    pub mint_freeze_authority: Option<Context<ConstraintMintFreezeAuthority>>,
    pub mint_decimals: Option<Context<ConstraintMintDecimals>>,
    pub mint_token_program: Option<Context<ConstraintTokenProgram>>,
    pub mint_compressed: Option<Context<ConstraintMintCompressed>>,
    pub extension_group_pointer_authority: Option<Context<ConstraintExtensionAuthority>>,
    pub extension_group_pointer_group_address:
        Option<Context<ConstraintExtensionGroupPointerGroupAddress>>,
    pub extension_group_member_pointer_authority: Option<Context<ConstraintExtensionAuthority>>,
    pub extension_group_member_pointer_member_address:
        Option<Context<ConstraintExtensionGroupMemberPointerMemberAddress>>,
    pub extension_metadata_pointer_authority: Option<Context<ConstraintExtensionAuthority>>,
    pub extension_metadata_pointer_metadata_address:
        Option<Context<ConstraintExtensionMetadataPointerMetadataAddress>>,
    pub extension_close_authority: Option<Context<ConstraintExtensionAuthority>>,
    pub extension_transfer_hook_authority: Option<Context<ConstraintExtensionAuthority>>,
    pub extension_transfer_hook_program_id: Option<Context<ConstraintExtensionTokenHookProgramId>>,
    pub extension_permanent_delegate: Option<Context<ConstraintExtensionPermanentDelegate>>,
    pub bump: Option<Context<ConstraintTokenBump>>,
    pub program_seed: Option<Context<ConstraintProgramSeed>>,
    pub realloc: Option<Context<ConstraintRealloc>>,
    pub realloc_payer: Option<Context<ConstraintReallocPayer>>,
    pub realloc_zero: Option<Context<ConstraintReallocZero>>,
    // CToken Mint constraints
    pub ctoken_mint_authority: Option<Context<ConstraintCTokenMintAuthority>>,
    pub ctoken_mint_freeze_authority: Option<Context<ConstraintCTokenMintFreezeAuthority>>,
    pub ctoken_mint_payer: Option<Context<ConstraintCTokenMintPayer>>,
    pub ctoken_mint_decimals: Option<Context<ConstraintCTokenMintDecimals>>,
    pub ctoken_mint_signer: Option<Context<ConstraintCTokenMintSigner>>,
    pub ctoken_mint_signer_seeds: Option<Context<ConstraintCTokenMintSignerSeeds>>,
    pub ctoken_mint_signer_bump: Option<Context<ConstraintCTokenMintSignerBump>>,
    pub ctoken_mint_program_authority_seeds: Option<Context<ConstraintCTokenMintProgramAuthoritySeeds>>,
    pub ctoken_mint_program_authority_bump: Option<Context<ConstraintCTokenMintProgramAuthorityBump>>,
    pub ctoken_mint_address_tree_info: Option<Context<ConstraintCTokenMintAddressTreeInfo>>,
    pub ctoken_mint_proof: Option<Context<ConstraintCTokenMintProof>>,
    pub ctoken_mint_output_state_tree_index: Option<Context<ConstraintCTokenMintOutputStateTreeIndex>>,
    // Metadata constraints (for CToken mints)
    pub metadata_name: Option<Context<ConstraintMetadataName>>,
    pub metadata_symbol: Option<Context<ConstraintMetadataSymbol>>,
    pub metadata_uri: Option<Context<ConstraintMetadataUri>>,
    pub metadata_update_authority: Option<Context<ConstraintMetadataUpdateAuthority>>,
    pub metadata_additional: Option<Context<ConstraintMetadataAdditional>>,
    // CPDA constraints
    pub cpda_authority: Option<Context<ConstraintCPDAAuthority>>,
    pub cpda_address_tree_info: Option<Context<ConstraintCPDAAddressTreeInfo>>,
    pub cpda_proof: Option<Context<ConstraintCPDAProof>>,
    pub cpda_output_state_tree_index: Option<Context<ConstraintCPDAOutputStateTreeIndex>>,
    pub cpda_compress_on_init: Option<Context<ConstraintCPDACompressOnInit>>,
    // CCToken constraint
    pub cctoken: Option<Context<ConstraintCCToken>>,
}

impl<'ty> ConstraintGroupBuilder<'ty> {
    pub fn new(f_ty: Option<&'ty Ty>) -> Self {
        Self {
            f_ty,
            instruction_api: None,
            init: None,
            zeroed: None,
            mutable: None,
            signer: None,
            has_one: Vec::new(),
            raw: Vec::new(),
            owner: None,
            rent_exempt: None,
            seeds: None,
            executable: None,
            payer: None,
            space: None,
            close: None,
            address: None,
            token_mint: None,
            token_authority: None,
            token_token_program: None,
            associated_token_mint: None,
            associated_token_authority: None,
            associated_token_token_program: None,
            mint_authority: None,
            mint_freeze_authority: None,
            mint_decimals: None,
            mint_token_program: None,
            mint_compressed: None,
            extension_group_pointer_authority: None,
            extension_group_pointer_group_address: None,
            extension_group_member_pointer_authority: None,
            extension_group_member_pointer_member_address: None,
            extension_metadata_pointer_authority: None,
            extension_metadata_pointer_metadata_address: None,
            extension_close_authority: None,
            extension_transfer_hook_authority: None,
            extension_transfer_hook_program_id: None,
            extension_permanent_delegate: None,
            bump: None,
            program_seed: None,
            realloc: None,
            realloc_payer: None,
            realloc_zero: None,
            ctoken_mint_authority: None,
            ctoken_mint_freeze_authority: None,
            ctoken_mint_payer: None,
            ctoken_mint_decimals: None,
            ctoken_mint_signer: None,
            ctoken_mint_signer_seeds: None,
            ctoken_mint_signer_bump: None,
            ctoken_mint_program_authority_seeds: None,
            ctoken_mint_program_authority_bump: None,
            ctoken_mint_address_tree_info: None,
            ctoken_mint_proof: None,
            ctoken_mint_output_state_tree_index: None,
            metadata_name: None,
            metadata_symbol: None,
            metadata_uri: None,
            metadata_update_authority: None,
            metadata_additional: None,
            cpda_authority: None,
            cpda_address_tree_info: None,
            cpda_proof: None,
            cpda_output_state_tree_index: None,
            cpda_compress_on_init: None,
            cctoken: None,
        }
    }
    
    pub fn set_instruction_api(&mut self, instruction_api: Option<Punctuated<Expr, Comma>>) {
        self.instruction_api = instruction_api;
    }
    
    fn auto_detect_compression_fields(&mut self) -> ParseResult<()> {
        // Only auto-detect if instruction data is available
        let instruction_api = match &self.instruction_api {
            Some(api) => api,
            None => return Ok(()),
        };
        
        // Helper to find a field in instruction parameters that contains a specific nested field
        let find_field_with_nested = |nested_field_name: &str| -> Option<(String, String)> {
            for expr in instruction_api.iter() {
                if let Expr::Type(expr_type) = expr {
                    let field_str = quote::quote! { #expr_type }.to_string();
                    // Parse pattern like "compression_params : InitializeCompressionParams"
                    let parts: Vec<&str> = field_str.split(" : ").collect();
                    if parts.len() == 2 {
                        let param_name = parts[0].trim();
                        // Check if this could be a struct containing our nested field
                        // We look for common patterns in the type name
                        let type_name = parts[1].trim();
                        if type_name.contains("Compression") || type_name.contains("Params") {
                            // Found a potential compression params struct
                            return Some((param_name.to_string(), nested_field_name.to_string()));
                        }
                    }
                }
            }
            None
        };
        
        // Helper to find ValidityProof field directly - returns Result to handle multiple proofs
        let find_validity_proof_field = || -> ParseResult<Option<String>> {
            let mut found_proofs = Vec::new();
            
            // First, look for direct ValidityProof fields
            for expr in instruction_api.iter() {
                if let Expr::Type(expr_type) = expr {
                    let field_str = quote::quote! { #expr_type }.to_string();
                    let parts: Vec<&str> = field_str.split(" : ").collect();
                    if parts.len() == 2 {
                        let param_name = parts[0].trim();
                        let type_name = parts[1].trim();
                        // Check if this is a ValidityProof type
                        if type_name == "ValidityProof" || type_name.ends_with("::ValidityProof") {
                            found_proofs.push(param_name.to_string());
                        }
                    }
                }
            }
            
            // If no direct ValidityProof, look for it in a compression params struct
            if found_proofs.is_empty() {
                if let Some((param, field)) = find_field_with_nested("proof") {
                    found_proofs.push(format!("{}.{}", param, field));
                }
            }
            
            // Check for ambiguity
            match found_proofs.len() {
                0 => Ok(None),
                1 => Ok(Some(found_proofs[0].clone())),
                _ => {
                    // Multiple proofs found - require explicit specification
                    Err(ParseError::new(
                        proc_macro2::Span::call_site(),
                        format!(
                            "Multiple ValidityProof fields found in instruction parameters: {}. \
                            Please explicitly specify the proof constraint (e.g., cpda::proof = compression_params.proof)",
                            found_proofs.join(", ")
                        )
                    ))
                }
            }
        };
        
        // Auto-detect for CPDA constraints if we have CPDA fields but missing some
        let has_cpda = self.cpda_authority.is_some() || 
                       self.cpda_address_tree_info.is_some() ||
                       self.cpda_output_state_tree_index.is_some() ||
                       self.cpda_compress_on_init.is_some();
                       
        if has_cpda {
            // Auto-detect proof if not explicitly set
            if self.cpda_proof.is_none() {
                match find_validity_proof_field()? {
                    Some(proof_field) => {
                        let proof_expr: Expr = syn::parse_str(&proof_field)
                            .map_err(|_| ParseError::new(
                                proc_macro2::Span::call_site(),
                                format!("Failed to parse auto-detected proof field: {}", proof_field)
                            ))?;
                        self.cpda_proof = Some(Context::new(
                            proc_macro2::Span::call_site(),
                            ConstraintCPDAProof { proof: proof_expr }
                        ));
                    }
                    None => {
                        // No proof found - this is an error for CPDA
                        return Err(ParseError::new(
                            proc_macro2::Span::call_site(),
                            "CPDA constraints require a proof field. Either add 'cpda::proof = <expr>' \
                            or ensure your instruction has a ValidityProof parameter."
                        ));
                    }
                }
            }
            
            // Default output_state_tree_index to 0 if not explicitly set
            if self.cpda_output_state_tree_index.is_none() {
                let default_index: Expr = syn::parse_str("0")
                    .expect("Failed to parse default output_state_tree_index");
                self.cpda_output_state_tree_index = Some(Context::new(
                    proc_macro2::Span::call_site(),
                    ConstraintCPDAOutputStateTreeIndex { output_state_tree_index: default_index }
                ));
            }
            
            // Auto-detect other fields if patterns are found
            // Look for fields that match common patterns
            for expr in instruction_api.iter() {
                if let Expr::Type(expr_type) = expr {
                    let field_str = quote::quote! { #expr_type }.to_string();
                    let parts: Vec<&str> = field_str.split(" : ").collect();
                    if parts.len() == 2 {
                        let param_name = parts[0].trim();
                        let type_name = parts[1].trim();
                        
                        // Auto-detect address_tree_info
                        if self.cpda_address_tree_info.is_none() && 
                           (type_name.contains("AddressTreeInfo") || type_name.contains("PackedAddressTreeInfo")) {
                            let field_expr: Expr = syn::parse_str(param_name)
                                .map_err(|_| ParseError::new(
                                    proc_macro2::Span::call_site(),
                                    format!("Failed to parse auto-detected address_tree_info: {}", param_name)
                                ))?;
                            self.cpda_address_tree_info = Some(Context::new(
                                proc_macro2::Span::call_site(),
                                ConstraintCPDAAddressTreeInfo { address_tree_info: field_expr }
                            ));
                        }
                    }
                }
            }
        }
        
        // Similar auto-detection for CToken Mint constraints
        let has_ctoken_mint = self.ctoken_mint_authority.is_some() ||
                        self.ctoken_mint_payer.is_some() ||
                        self.ctoken_mint_decimals.is_some() ||
                        self.ctoken_mint_signer.is_some() ||
                        self.ctoken_mint_address_tree_info.is_some() ||
                        self.ctoken_mint_output_state_tree_index.is_some();
                        
        if has_ctoken_mint {
            // Auto-detect proof if not explicitly set
            if self.ctoken_mint_proof.is_none() {
                match find_validity_proof_field()? {
                    Some(proof_field) => {
                        let proof_expr: Expr = syn::parse_str(&proof_field)
                            .map_err(|_| ParseError::new(
                                proc_macro2::Span::call_site(),
                                format!("Failed to parse auto-detected proof field for CToken Mint: {}", proof_field)
                            ))?;
                        self.ctoken_mint_proof = Some(Context::new(
                            proc_macro2::Span::call_site(),
                            ConstraintCTokenMintProof { proof: proof_expr }
                        ));
                    }
                    None => {
                        // No proof found - this is an error for CToken Mint
                        return Err(ParseError::new(
                            proc_macro2::Span::call_site(),
                            "CToken Mint constraints require a proof field. Either add 'mint::proof = <expr>' \
                            or ensure your instruction has a ValidityProof parameter."
                        ));
                    }
                }
            }
            
            // Default output_state_tree_index to 0 if not explicitly set
            if self.ctoken_mint_output_state_tree_index.is_none() {
                let default_index: Expr = syn::parse_str("0")
                    .expect("Failed to parse default output_state_tree_index");
                self.ctoken_mint_output_state_tree_index = Some(Context::new(
                    proc_macro2::Span::call_site(),
                    ConstraintCTokenMintOutputStateTreeIndex { output_state_tree_index: default_index }
                ));
            }
        }
        
        Ok(())
    }

    pub fn build(mut self) -> ParseResult<ConstraintGroup> {
        // Init.
        if let Some(i) = &self.init {
            if cfg!(not(feature = "init-if-needed")) && i.if_needed {
                return Err(ParseError::new(
                    i.span(),
                    "init_if_needed requires that anchor-lang be imported \
                    with the init-if-needed cargo feature enabled. \
                    Carefully read the init_if_needed docs before using this feature \
                    to make sure you know how to protect yourself against \
                    re-initialization attacks.",
                ));
            }

            match self.mutable {
                Some(m) => {
                    return Err(ParseError::new(
                        m.span(),
                        "mut cannot be provided with init",
                    ))
                }
                None => self
                    .mutable
                    .replace(Context::new(i.span(), ConstraintMut { error: None })),
            };
            // Rent exempt if not explicitly skipped.
            if self.rent_exempt.is_none() {
                // For compressed mints, we do not create a system account, so skip rent check by default.
                let is_compressed_mint = self.mint_decimals.is_some()
                    && self
                        .mint_compressed
                        .as_ref()
                        .map(|c| c.inner.compressed)
                        .unwrap_or(false);
                let rent_policy = if is_compressed_mint {
                    ConstraintRentExempt::Skip
                } else {
                    ConstraintRentExempt::Enforce
                };
                self.rent_exempt
                    .replace(Context::new(i.span(), rent_policy));
            }
            if self.payer.is_none() {
                return Err(ParseError::new(
                    i.span(),
                    "payer must be provided when initializing an account",
                ));
            }
            // When initializing a non-PDA account, the account being
            // initialized must sign to invoke the system program's create
            // account instruction.
            if self.signer.is_none() && self.seeds.is_none() && self.associated_token_mint.is_none()
            {
                self.signer
                    .replace(Context::new(i.span(), ConstraintSigner { error: None }));
            }

            // Assert a bump target is not given on init.
            if let Some(b) = &self.bump {
                if b.bump.is_some() {
                    return Err(ParseError::new(
                        b.span(),
                        "bump targets should not be provided with init. Please use bump without a target."
                    ));
                }
            }

            // TokenAccount.
            if let Some(token_mint) = &self.token_mint {
                if self.token_authority.is_none() {
                    return Err(ParseError::new(
                        token_mint.span(),
                        "when initializing, token authority must be provided if token mint is",
                    ));
                }
            }
            if let Some(token_authority) = &self.token_authority {
                if self.token_mint.is_none() {
                    return Err(ParseError::new(
                        token_authority.span(),
                        "when initializing, token mint must be provided if token authority is",
                    ));
                }
            }

            // Mint.
            if let Some(mint_decimals) = &self.mint_decimals {
                if self.mint_authority.is_none() {
                    return Err(ParseError::new(
                        mint_decimals.span(),
                        "when initializing, mint authority must be provided if mint decimals is",
                    ));
                }
            }
            if let Some(mint_authority) = &self.mint_authority {
                if self.mint_decimals.is_none() {
                    return Err(ParseError::new(
                        mint_authority.span(),
                        "when initializing, mint decimals must be provided if mint authority is",
                    ));
                }
            }
        }

        // Realloc.
        if let Some(r) = &self.realloc {
            if self.realloc_payer.is_none() {
                return Err(ParseError::new(
                    r.span(),
                    "realloc::payer must be provided when using realloc",
                ));
            }
            if self.realloc_zero.is_none() {
                return Err(ParseError::new(
                    r.span(),
                    "realloc::zero must be provided when using realloc",
                ));
            }
        }

        // Zero.
        if let Some(z) = &self.zeroed {
            match self.mutable {
                Some(m) => {
                    return Err(ParseError::new(
                        m.span(),
                        "mut cannot be provided with zeroed",
                    ))
                }
                None => self
                    .mutable
                    .replace(Context::new(z.span(), ConstraintMut { error: None })),
            };
            // Rent exempt if not explicitly skipped.
            if self.rent_exempt.is_none() {
                self.rent_exempt
                    .replace(Context::new(z.span(), ConstraintRentExempt::Enforce));
            }
        }

        // Seeds.
        if let Some(i) = &self.seeds {
            if self.init.is_some() && self.payer.is_none() {
                return Err(ParseError::new(
                    i.span(),
                    "payer must be provided when creating a program derived address",
                ));
            }
            if self.bump.is_none() {
                return Err(ParseError::new(
                    i.span(),
                    "bump must be provided with seeds",
                ));
            }
        }

        // Space.
        if let Some(i) = &self.init {
            let initializing_token_program_acc = self.token_mint.is_some()
                || self.mint_authority.is_some()
                || self.token_authority.is_some()
                || self.associated_token_authority.is_some();

            match (self.space.is_some(), initializing_token_program_acc) {
                (true, true) => {
                    return Err(ParseError::new(
                        self.space.as_ref().unwrap().span(),
                        "space is not required for initializing an spl account",
                    ));
                }
                (false, false) => {
                    return Err(ParseError::new(
                        i.span(),
                        "space must be provided with init",
                    ));
                }
                _ => (),
            }
        }

        // Auto-detect compression fields from instruction data if not explicitly provided
        self.auto_detect_compression_fields()?;
        
        let ConstraintGroupBuilder {
            f_ty: _,
            instruction_api: _,
            init,
            zeroed,
            mutable,
            signer,
            has_one,
            raw,
            owner,
            rent_exempt,
            seeds,
            executable,
            payer,
            space,
            close,
            address,
            token_mint,
            token_authority,
            token_token_program,
            associated_token_mint,
            associated_token_authority,
            associated_token_token_program,
            mint_authority,
            mint_freeze_authority,
            mint_decimals,
            mint_token_program,
            mint_compressed,
            extension_group_pointer_authority,
            extension_group_pointer_group_address,
            extension_group_member_pointer_authority,
            extension_group_member_pointer_member_address,
            extension_metadata_pointer_authority,
            extension_metadata_pointer_metadata_address,
            extension_close_authority,
            extension_transfer_hook_authority,
            extension_transfer_hook_program_id,
            extension_permanent_delegate,
            bump,
            program_seed,
            realloc,
            realloc_payer,
            realloc_zero,
            ctoken_mint_authority,
            ctoken_mint_freeze_authority,
            ctoken_mint_payer,
            ctoken_mint_decimals,
            ctoken_mint_signer,
            ctoken_mint_signer_seeds,
            ctoken_mint_signer_bump,
            ctoken_mint_program_authority_seeds,
            ctoken_mint_program_authority_bump,
            ctoken_mint_address_tree_info,
            ctoken_mint_proof,
            ctoken_mint_output_state_tree_index,
            ref metadata_name,
            ref metadata_symbol,
            ref metadata_uri,
            ref metadata_update_authority,
            ref metadata_additional,
            cpda_authority,
            cpda_address_tree_info,
            cpda_proof,
            cpda_output_state_tree_index,
            cpda_compress_on_init,
            cctoken,
        } = self;

        // Converts Option<Context<T>> -> Option<T>.
        macro_rules! into_inner {
            ($opt:ident) => {
                $opt.map(|c| c.into_inner())
            };
            ($opt:expr) => {
                $opt.map(|c| c.into_inner())
            };
        }
        // Converts Vec<Context<T>> - Vec<T>.
        macro_rules! into_inner_vec {
            ($opt:ident) => {
                $opt.into_iter().map(|c| c.into_inner()).collect()
            };
        }

        let is_init = init.is_some();
        let seeds = seeds.map(|c| ConstraintSeedsGroup {
            is_init,
            seeds: c.seeds.clone(),
            bump: into_inner!(bump)
                .map(|b| b.bump)
                .expect("bump must be provided with seeds"),
            program_seed: into_inner!(program_seed).map(|id| id.program_seed),
        });
        let associated_token = match (
            associated_token_mint,
            associated_token_authority,
            &associated_token_token_program,
        ) {
            (Some(mint), Some(auth), _) => Some(ConstraintAssociatedToken {
                wallet: auth.into_inner().auth,
                mint: mint.into_inner().mint,
                token_program: associated_token_token_program
                    .as_ref()
                    .map(|a| a.clone().into_inner().token_program),
            }),
            (Some(mint), None, _) => return Err(ParseError::new(
                mint.span(),
                "authority must be provided to specify an associated token program derived address",
            )),
            (None, Some(auth), _) => {
                return Err(ParseError::new(
                    auth.span(),
                    "mint must be provided to specify an associated token program derived address",
                ))
            },
            (None, None, Some(token_program)) => {
                return Err(ParseError::new(
                    token_program.span(),
                    "mint and authority must be provided to specify an associated token program derived address",
                ))
            }
            _ => None,
        };
        if let Some(associated_token) = &associated_token {
            if seeds.is_some() {
                return Err(ParseError::new(
                    associated_token.mint.span(),
                    "'associated_token' constraints cannot be used with the 'seeds' constraint",
                ));
            }
        }

        let token_account = match (&token_mint, &token_authority, &token_token_program) {
            (None, None, None) => None,
            _ => Some(ConstraintTokenAccountGroup {
                mint: token_mint.as_ref().map(|a| a.clone().into_inner().mint),
                authority: token_authority
                    .as_ref()
                    .map(|a| a.clone().into_inner().auth),
                token_program: token_token_program
                    .as_ref()
                    .map(|a| a.clone().into_inner().token_program),
            }),
        };

        let mint = match (
            &mint_decimals,
            &mint_authority,
            &mint_freeze_authority,
            &mint_token_program,
            &mint_compressed,
            &extension_group_pointer_authority,
            &extension_group_pointer_group_address,
            &extension_group_member_pointer_authority,
            &extension_group_member_pointer_member_address,
            &extension_metadata_pointer_authority,
            &extension_metadata_pointer_metadata_address,
            &extension_close_authority,
            &extension_transfer_hook_authority,
            &extension_transfer_hook_program_id,
            &extension_permanent_delegate,
        ) {
            (
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            ) => None,
            _ => Some(ConstraintTokenMintGroup {
                decimals: mint_decimals
                    .as_ref()
                    .map(|a| a.clone().into_inner().decimals),
                mint_authority: mint_authority
                    .as_ref()
                    .map(|a| a.clone().into_inner().mint_auth),
                freeze_authority: mint_freeze_authority
                    .as_ref()
                    .map(|a| a.clone().into_inner().mint_freeze_auth),
                token_program: mint_token_program
                    .as_ref()
                    .map(|a| a.clone().into_inner().token_program),
                compressed: mint_compressed
                    .as_ref()
                    .map(|a| a.clone().into_inner().compressed),
                // extensions
                group_pointer_authority: extension_group_pointer_authority
                    .as_ref()
                    .map(|a| a.clone().into_inner().authority),
                group_pointer_group_address: extension_group_pointer_group_address
                    .as_ref()
                    .map(|a| a.clone().into_inner().group_address),
                group_member_pointer_authority: extension_group_member_pointer_authority
                    .as_ref()
                    .map(|a| a.clone().into_inner().authority),
                group_member_pointer_member_address: extension_group_member_pointer_member_address
                    .as_ref()
                    .map(|a| a.clone().into_inner().member_address),
                metadata_pointer_authority: extension_metadata_pointer_authority
                    .as_ref()
                    .map(|a| a.clone().into_inner().authority),
                metadata_pointer_metadata_address: extension_metadata_pointer_metadata_address
                    .as_ref()
                    .map(|a| a.clone().into_inner().metadata_address),
                close_authority: extension_close_authority
                    .as_ref()
                    .map(|a| a.clone().into_inner().authority),
                permanent_delegate: extension_permanent_delegate
                    .as_ref()
                    .map(|a| a.clone().into_inner().permanent_delegate),
                transfer_hook_authority: extension_transfer_hook_authority
                    .as_ref()
                    .map(|a| a.clone().into_inner().authority),
                transfer_hook_program_id: extension_transfer_hook_program_id
                    .as_ref()
                    .map(|a| a.clone().into_inner().program_id),
            }),
        };

        Ok(ConstraintGroup {
            init: init.as_ref().map(|i| Ok(ConstraintInitGroup {
                if_needed: i.if_needed,
                seeds: seeds.clone(),
                payer: into_inner!(payer.clone()).unwrap().target,
                space: space.clone().map(|s| s.space.clone()),
                kind: if let Some(tm) = &token_mint {
                    InitKind::Token {
                        mint: tm.clone().into_inner().mint,
                        owner: match &token_authority {
                            Some(a) => a.clone().into_inner().auth,
                            None => return Err(ParseError::new(
                                tm.span(),
                                "authority must be provided to initialize a token program derived address"
                            )),
                        },
                        token_program: token_token_program.map(|tp| tp.into_inner().token_program),
                    }
                } else if let Some(at) = &associated_token {
                    InitKind::AssociatedToken {
                        mint: at.mint.clone(),
                        owner: at.wallet.clone(),
                        token_program: associated_token_token_program.map(|tp| tp.into_inner().token_program),
                    }
                } else if let Some(d) = &mint_decimals {
                    InitKind::Mint {
                        decimals: d.clone().into_inner().decimals,
                        owner: match &mint_authority {
                            Some(a) => a.clone().into_inner().mint_auth,
                            None => return Err(ParseError::new(
                                d.span(),
                                "authority must be provided to initialize a mint program derived address"
                            ))
                        },
                        freeze_authority: mint_freeze_authority.map(|fa| fa.into_inner().mint_freeze_auth),
                        token_program: mint_token_program.map(|tp| tp.into_inner().token_program),
                        compressed: mint_compressed.map(|mc| mc.into_inner().compressed),
                        // extensions
                        group_pointer_authority: extension_group_pointer_authority.map(|gpa| gpa.into_inner().authority),
                        group_pointer_group_address: extension_group_pointer_group_address.map(|gpga| gpga.into_inner().group_address),
                        group_member_pointer_authority: extension_group_member_pointer_authority.map(|gmpa| gmpa.into_inner().authority),
                        group_member_pointer_member_address: extension_group_member_pointer_member_address.map(|gmpma| gmpma.into_inner().member_address),
                        metadata_pointer_authority: extension_metadata_pointer_authority.map(|mpa| mpa.into_inner().authority),
                        metadata_pointer_metadata_address: extension_metadata_pointer_metadata_address.map(|mpma| mpma.into_inner().metadata_address),
                        close_authority: extension_close_authority.map(|ca| ca.into_inner().authority),
                        permanent_delegate: extension_permanent_delegate.map(|pd| pd.into_inner().permanent_delegate),
                        transfer_hook_authority: extension_transfer_hook_authority.map(|tha| tha.into_inner().authority),
                        transfer_hook_program_id: extension_transfer_hook_program_id.map(|thpid| thpid.into_inner().program_id),
                    }
                } else {
                    InitKind::Program {
                        owner: owner.as_ref().map(|o| o.owner_address.clone()),
                    }
                },
            })).transpose()?,
            realloc: realloc.as_ref().map(|r| ConstraintReallocGroup {
                payer: into_inner!(realloc_payer).unwrap().target,
                space: r.space.clone(),
                zero: into_inner!(realloc_zero).unwrap().zero,
            }),
            zeroed: into_inner!(zeroed),
            mutable: into_inner!(mutable),
            signer: into_inner!(signer),
            has_one: into_inner_vec!(has_one),
            raw: into_inner_vec!(raw),
            owner: into_inner!(owner),
            rent_exempt: into_inner!(rent_exempt),
            executable: into_inner!(executable),
            close: into_inner!(close),
            address: into_inner!(address),
            associated_token: if !is_init { associated_token } else { None },
            seeds,
            token_account: if !is_init {token_account} else {None},
            mint: if !is_init {mint} else {None},
            cmint: if ctoken_mint_authority.is_some() || 
                       ctoken_mint_freeze_authority.is_some() || metadata_name.is_some() || metadata_symbol.is_some() || metadata_uri.is_some() || metadata_update_authority.is_some() || metadata_additional.is_some() || ctoken_mint_payer.is_some() || ctoken_mint_decimals.is_some() || 
                       ctoken_mint_signer.is_some() || ctoken_mint_signer_seeds.is_some() || 
                       ctoken_mint_signer_bump.is_some() ||
                       ctoken_mint_program_authority_seeds.is_some() || ctoken_mint_program_authority_bump.is_some() ||
                       ctoken_mint_address_tree_info.is_some() || ctoken_mint_proof.is_some() || 
                       ctoken_mint_output_state_tree_index.is_some() {
                Some(ConstraintCTokenMintGroup {
                    authority: ctoken_mint_authority.map(|c| c.into_inner().authority),
                    metadata_name: metadata_name.as_ref().map(|c| c.clone().into_inner().name),
                    metadata_symbol: metadata_symbol.as_ref().map(|c| c.clone().into_inner().symbol),
                    metadata_uri: metadata_uri.as_ref().map(|c| c.clone().into_inner().uri),
                    metadata_update_authority: metadata_update_authority.as_ref().map(|c| c.clone().into_inner().update_authority),
                    metadata_additional: metadata_additional.as_ref().map(|c| c.clone().into_inner().additional),
                    freeze_authority: ctoken_mint_freeze_authority.map(|c| c.into_inner().freeze_authority),
                    decimals: ctoken_mint_decimals.map(|c| c.into_inner().decimals),
                    payer: ctoken_mint_payer.map(|c| c.into_inner().payer),
                    mint_signer: ctoken_mint_signer.map(|c| c.into_inner().signer),
                    mint_signer_seeds: ctoken_mint_signer_seeds.map(|c| c.into_inner().seeds),
                    mint_signer_bump: ctoken_mint_signer_bump.map(|c| c.into_inner().bump),
                    program_authority_seeds: ctoken_mint_program_authority_seeds.map(|c| c.into_inner().seeds),
                    program_authority_bump: ctoken_mint_program_authority_bump.map(|c| c.into_inner().bump),
                    address_tree_info: ctoken_mint_address_tree_info.map(|c| c.into_inner().address_tree_info),
                    proof: ctoken_mint_proof.map(|c| c.into_inner().proof),
                    output_state_tree_index: ctoken_mint_output_state_tree_index.map(|c| c.into_inner().output_state_tree_index),
                })
            } else {
                None
            },
            cpda: if cpda_authority.is_some() || cpda_address_tree_info.is_some() || cpda_proof.is_some() || 
                     cpda_output_state_tree_index.is_some() || cpda_compress_on_init.is_some() {
                Some(ConstraintCPDAGroup {
                    compress_on_init: cpda_compress_on_init.is_some(),
                    authority: cpda_authority.map(|c| c.into_inner().authority),
                    address_tree_info: cpda_address_tree_info.map(|c| c.into_inner().address_tree_info),
                    proof: cpda_proof.map(|c| c.into_inner().proof),
                    output_state_tree_index: cpda_output_state_tree_index.map(|c| c.into_inner().output_state_tree_index),
                })
            } else {
                None
            },
            cctoken: cctoken.map(|c| ConstraintCCTokenGroup {
                mint: c.into_inner().mint,
            }),
        })
    }

    pub fn add(&mut self, c: ConstraintToken) -> ParseResult<()> {
        match c {
            ConstraintToken::Init(c) => self.add_init(c),
            ConstraintToken::Zeroed(c) => self.add_zeroed(c),
            ConstraintToken::Mut(c) => self.add_mut(c),
            ConstraintToken::Signer(c) => self.add_signer(c),
            ConstraintToken::HasOne(c) => self.add_has_one(c),
            ConstraintToken::Raw(c) => self.add_raw(c),
            ConstraintToken::Owner(c) => self.add_owner(c),
            ConstraintToken::RentExempt(c) => self.add_rent_exempt(c),
            ConstraintToken::Seeds(c) => self.add_seeds(c),
            ConstraintToken::Executable(c) => self.add_executable(c),
            ConstraintToken::Payer(c) => self.add_payer(c),
            ConstraintToken::Space(c) => self.add_space(c),
            ConstraintToken::Close(c) => self.add_close(c),
            ConstraintToken::Address(c) => self.add_address(c),
            ConstraintToken::TokenAuthority(c) => self.add_token_authority(c),
            ConstraintToken::TokenMint(c) => self.add_token_mint(c),
            ConstraintToken::TokenTokenProgram(c) => self.add_token_token_program(c),
            ConstraintToken::AssociatedTokenAuthority(c) => self.add_associated_token_authority(c),
            ConstraintToken::AssociatedTokenMint(c) => self.add_associated_token_mint(c),
            ConstraintToken::AssociatedTokenTokenProgram(c) => {
                self.add_associated_token_token_program(c)
            }
            ConstraintToken::MintAuthority(c) => self.add_mint_authority(c),
            ConstraintToken::MintFreezeAuthority(c) => self.add_mint_freeze_authority(c),
            ConstraintToken::MintDecimals(c) => self.add_mint_decimals(c),
            ConstraintToken::MintTokenProgram(c) => self.add_mint_token_program(c),
            ConstraintToken::MintCompressed(c) => self.add_mint_compressed(c),
            ConstraintToken::Bump(c) => self.add_bump(c),
            ConstraintToken::ProgramSeed(c) => self.add_program_seed(c),
            ConstraintToken::Realloc(c) => self.add_realloc(c),
            ConstraintToken::ReallocPayer(c) => self.add_realloc_payer(c),
            ConstraintToken::ReallocZero(c) => self.add_realloc_zero(c),
            ConstraintToken::ExtensionGroupPointerAuthority(c) => {
                self.add_extension_group_pointer_authority(c)
            }
            ConstraintToken::ExtensionGroupPointerGroupAddress(c) => {
                self.add_extension_group_pointer_group_address(c)
            }
            ConstraintToken::ExtensionGroupMemberPointerAuthority(c) => {
                self.add_extension_group_member_pointer_authority(c)
            }
            ConstraintToken::ExtensionGroupMemberPointerMemberAddress(c) => {
                self.add_extension_group_member_pointer_member_address(c)
            }
            ConstraintToken::ExtensionMetadataPointerAuthority(c) => {
                self.add_extension_metadata_pointer_authority(c)
            }
            ConstraintToken::ExtensionMetadataPointerMetadataAddress(c) => {
                self.add_extension_metadata_pointer_metadata_address(c)
            }
            ConstraintToken::ExtensionCloseAuthority(c) => self.add_extension_close_authority(c),
            ConstraintToken::ExtensionTokenHookAuthority(c) => self.add_extension_authority(c),
            ConstraintToken::ExtensionTokenHookProgramId(c) => {
                self.add_extension_transfer_hook_program_id(c)
            }
            ConstraintToken::ExtensionPermanentDelegate(c) => {
                self.add_extension_permanent_delegate(c)
            }
            ConstraintToken::CTokenMintAuthority(c) => self.add_ctoken_mint_authority(c),
            ConstraintToken::CTokenMintFreezeAuthority(c) => self.add_ctoken_mint_freeze_authority(c),
            ConstraintToken::CTokenMintPayer(c) => self.add_ctoken_mint_payer(c),
            ConstraintToken::CTokenMintDecimals(c) => self.add_ctoken_mint_decimals(c),
            ConstraintToken::CTokenMintSigner(c) => self.add_ctoken_mint_signer(c),
            ConstraintToken::CTokenMintSignerSeeds(c) => self.add_ctoken_mint_signer_seeds(c),
            ConstraintToken::CTokenMintSignerBump(c) => self.add_ctoken_mint_signer_bump(c),
            ConstraintToken::CTokenMintProgramAuthoritySeeds(c) => self.add_ctoken_mint_program_authority_seeds(c),
            ConstraintToken::CTokenMintProgramAuthorityBump(c) => self.add_ctoken_mint_program_authority_bump(c),
            ConstraintToken::CTokenMintAddressTreeInfo(c) => self.add_ctoken_mint_address_tree_info(c),
            ConstraintToken::CTokenMintProof(c) => self.add_ctoken_mint_proof(c),
            ConstraintToken::CTokenMintOutputStateTreeIndex(c) => self.add_ctoken_mint_output_state_tree_index(c),
            ConstraintToken::MetadataName(c) => self.add_metadata_name(c),
            ConstraintToken::MetadataSymbol(c) => self.add_metadata_symbol(c),
            ConstraintToken::MetadataUri(c) => self.add_metadata_uri(c),
            ConstraintToken::MetadataUpdateAuthority(c) => self.add_metadata_update_authority(c),
            ConstraintToken::MetadataAdditional(c) => self.add_metadata_additional(c),
            ConstraintToken::CPDAAuthority(c) => self.add_cpda_authority(c),
            ConstraintToken::CPDAAddressTreeInfo(c) => self.add_cpda_address_tree_info(c),
            ConstraintToken::CPDAProof(c) => self.add_cpda_proof(c),
            ConstraintToken::CPDAOutputStateTreeIndex(c) => self.add_cpda_output_state_tree_index(c),
            ConstraintToken::CPDACompressOnInit(c) => self.add_cpda_compress_on_init(c),
            ConstraintToken::CCToken(c) => self.add_cctoken(c),
        }
    }

    fn add_init(&mut self, c: Context<ConstraintInit>) -> ParseResult<()> {
        if let Some(existing) = &mut self.init {
            // Merge if_needed flag if different
            if c.inner.if_needed != existing.inner.if_needed {
                existing.inner.if_needed = c.inner.if_needed;
                return Ok(());
            }
            return Err(ParseError::new(c.span(), "init already provided"));
        }
        if self.zeroed.is_some() {
            return Err(ParseError::new(c.span(), "zeroed already provided"));
        }
        if self.token_mint.is_some() {
            return Err(ParseError::new(
                c.span(),
                "init must be provided before token mint",
            ));
        }
        if self.token_authority.is_some() {
            return Err(ParseError::new(
                c.span(),
                "init must be provided before token authority",
            ));
        }
        if self.token_token_program.is_some() {
            return Err(ParseError::new(
                c.span(),
                "init must be provided before token account token program",
            ));
        }
        if self.mint_authority.is_some() {
            return Err(ParseError::new(
                c.span(),
                "init must be provided before mint authority",
            ));
        }
        if self.mint_freeze_authority.is_some() {
            return Err(ParseError::new(
                c.span(),
                "init must be provided before mint freeze authority",
            ));
        }
        if self.mint_decimals.is_some() {
            return Err(ParseError::new(
                c.span(),
                "init must be provided before mint decimals",
            ));
        }
        if self.mint_token_program.is_some() {
            return Err(ParseError::new(
                c.span(),
                "init must be provided before mint token program",
            ));
        }
        if self.associated_token_mint.is_some() {
            return Err(ParseError::new(
                c.span(),
                "init must be provided before associated token mint",
            ));
        }
        if self.associated_token_authority.is_some() {
            return Err(ParseError::new(
                c.span(),
                "init must be provided before associated token authority",
            ));
        }
        if self.associated_token_token_program.is_some() {
            return Err(ParseError::new(
                c.span(),
                "init must be provided before associated token account token program",
            ));
        }
        self.init.replace(c);
        Ok(())
    }

    fn add_zeroed(&mut self, c: Context<ConstraintZeroed>) -> ParseResult<()> {
        if self.zeroed.is_some() {
            return Err(ParseError::new(c.span(), "zeroed already provided"));
        }
        if self.init.is_some() {
            return Err(ParseError::new(c.span(), "init already provided"));
        }

        // Require a known account type that implements the `Discriminator` trait so that we can
        // get the discriminator length dynamically
        if !matches!(
            &self.f_ty,
            Some(Ty::Account(_) | Ty::LazyAccount(_) | Ty::AccountLoader(_))
        ) {
            return Err(ParseError::new(
                c.span(),
                "`zero` constraint requires the type to implement the `Discriminator` trait",
            ));
        }
        self.zeroed.replace(c);
        Ok(())
    }

    fn add_realloc(&mut self, c: Context<ConstraintRealloc>) -> ParseResult<()> {
        if !matches!(self.f_ty, Some(Ty::Account(_)))
            && !matches!(self.f_ty, Some(Ty::LazyAccount(_)))
            && !matches!(self.f_ty, Some(Ty::AccountLoader(_)))
        {
            return Err(ParseError::new(
                c.span(),
                "realloc must be on an Account, LazyAccount or AccountLoader",
            ));
        }
        if self.mutable.is_none() {
            return Err(ParseError::new(
                c.span(),
                "mut must be provided before realloc",
            ));
        }
        if self.realloc.is_some() {
            return Err(ParseError::new(c.span(), "realloc already provided"));
        }
        self.realloc.replace(c);
        Ok(())
    }

    fn add_realloc_payer(&mut self, c: Context<ConstraintReallocPayer>) -> ParseResult<()> {
        if self.realloc.is_none() {
            return Err(ParseError::new(
                c.span(),
                "realloc must be provided before realloc::payer",
            ));
        }
        if self.realloc_payer.is_some() {
            return Err(ParseError::new(c.span(), "realloc::payer already provided"));
        }
        self.realloc_payer.replace(c);
        Ok(())
    }

    fn add_realloc_zero(&mut self, c: Context<ConstraintReallocZero>) -> ParseResult<()> {
        if self.realloc.is_none() {
            return Err(ParseError::new(
                c.span(),
                "realloc must be provided before realloc::zero",
            ));
        }
        if self.realloc_zero.is_some() {
            return Err(ParseError::new(c.span(), "realloc::zero already provided"));
        }
        self.realloc_zero.replace(c);
        Ok(())
    }

    fn add_close(&mut self, c: Context<ConstraintClose>) -> ParseResult<()> {
        if !matches!(self.f_ty, Some(Ty::Account(_)))
            && !matches!(self.f_ty, Some(Ty::LazyAccount(_)))
            && !matches!(self.f_ty, Some(Ty::AccountLoader(_)))
        {
            return Err(ParseError::new(
                c.span(),
                "close must be on an Account, LazyAccount or AccountLoader",
            ));
        }
        if self.mutable.is_none() {
            return Err(ParseError::new(
                c.span(),
                "mut must be provided before close",
            ));
        }
        if self.close.is_some() {
            return Err(ParseError::new(c.span(), "close already provided"));
        }
        self.close.replace(c);
        Ok(())
    }

    fn add_address(&mut self, c: Context<ConstraintAddress>) -> ParseResult<()> {
        if self.address.is_some() {
            return Err(ParseError::new(c.span(), "address already provided"));
        }
        self.address.replace(c);
        Ok(())
    }

    fn add_token_mint(&mut self, c: Context<ConstraintTokenMint>) -> ParseResult<()> {
        if self.token_mint.is_some() {
            return Err(ParseError::new(c.span(), "token mint already provided"));
        }
        if self.associated_token_mint.is_some() {
            return Err(ParseError::new(
                c.span(),
                "associated token mint already provided",
            ));
        }
        self.token_mint.replace(c);
        Ok(())
    }

    fn add_associated_token_mint(&mut self, c: Context<ConstraintTokenMint>) -> ParseResult<()> {
        if self.associated_token_mint.is_some() {
            return Err(ParseError::new(
                c.span(),
                "associated token mint already provided",
            ));
        }
        if self.token_mint.is_some() {
            return Err(ParseError::new(c.span(), "token mint already provided"));
        }
        self.associated_token_mint.replace(c);
        Ok(())
    }

    fn add_bump(&mut self, c: Context<ConstraintTokenBump>) -> ParseResult<()> {
        if self.bump.is_some() {
            return Err(ParseError::new(c.span(), "bump already provided"));
        }
        if self.seeds.is_none() {
            return Err(ParseError::new(
                c.span(),
                "seeds must be provided before bump",
            ));
        }
        self.bump.replace(c);
        Ok(())
    }

    fn add_program_seed(&mut self, c: Context<ConstraintProgramSeed>) -> ParseResult<()> {
        if self.program_seed.is_some() {
            return Err(ParseError::new(c.span(), "seeds::program already provided"));
        }
        if self.seeds.is_none() {
            return Err(ParseError::new(
                c.span(),
                "seeds must be provided before seeds::program",
            ));
        }
        if let Some(ref init) = self.init {
            if init.if_needed {
                return Err(ParseError::new(
                    c.span(),
                    "seeds::program cannot be used with init_if_needed",
                ));
            } else {
                return Err(ParseError::new(
                    c.span(),
                    "seeds::program cannot be used with init",
                ));
            }
        }
        self.program_seed.replace(c);
        Ok(())
    }

    fn add_token_authority(&mut self, c: Context<ConstraintTokenAuthority>) -> ParseResult<()> {
        if self.token_authority.is_some() {
            return Err(ParseError::new(
                c.span(),
                "token authority already provided",
            ));
        }
        self.token_authority.replace(c);
        Ok(())
    }

    fn add_associated_token_authority(
        &mut self,
        c: Context<ConstraintTokenAuthority>,
    ) -> ParseResult<()> {
        if self.associated_token_authority.is_some() {
            return Err(ParseError::new(
                c.span(),
                "associated token authority already provided",
            ));
        }
        if self.token_authority.is_some() {
            return Err(ParseError::new(
                c.span(),
                "token authority already provided",
            ));
        }
        self.associated_token_authority.replace(c);
        Ok(())
    }

    fn add_token_token_program(&mut self, c: Context<ConstraintTokenProgram>) -> ParseResult<()> {
        if self.token_token_program.is_some() {
            return Err(ParseError::new(
                c.span(),
                "token token_program already provided",
            ));
        }
        self.token_token_program.replace(c);
        Ok(())
    }

    fn add_associated_token_token_program(
        &mut self,
        c: Context<ConstraintTokenProgram>,
    ) -> ParseResult<()> {
        if self.associated_token_token_program.is_some() {
            return Err(ParseError::new(
                c.span(),
                "associated token token_program already provided",
            ));
        }
        self.associated_token_token_program.replace(c);
        Ok(())
    }

    fn add_mint_authority(&mut self, c: Context<ConstraintMintAuthority>) -> ParseResult<()> {
        if self.mint_authority.is_some() {
            return Err(ParseError::new(c.span(), "mint authority already provided"));
        }
        self.mint_authority.replace(c);
        Ok(())
    }

    fn add_mint_freeze_authority(
        &mut self,
        c: Context<ConstraintMintFreezeAuthority>,
    ) -> ParseResult<()> {
        if self.mint_freeze_authority.is_some() {
            return Err(ParseError::new(
                c.span(),
                "mint freeze_authority already provided",
            ));
        }
        self.mint_freeze_authority.replace(c);
        Ok(())
    }

    fn add_mint_decimals(&mut self, c: Context<ConstraintMintDecimals>) -> ParseResult<()> {
        if self.mint_decimals.is_some() {
            return Err(ParseError::new(c.span(), "mint decimals already provided"));
        }
        self.mint_decimals.replace(c);
        Ok(())
    }

    fn add_mint_token_program(&mut self, c: Context<ConstraintTokenProgram>) -> ParseResult<()> {
        if self.mint_token_program.is_some() {
            return Err(ParseError::new(
                c.span(),
                "mint token_program already provided",
            ));
        }
        self.mint_token_program.replace(c);
        Ok(())
    }

    fn add_mint_compressed(&mut self, c: Context<ConstraintMintCompressed>) -> ParseResult<()> {
        if self.mint_compressed.is_some() {
            return Err(ParseError::new(
                c.span(),
                "mint compressed already provided",
            ));
        }
        self.mint_compressed.replace(c);
        Ok(())
    }

    fn add_ctoken_mint_authority(&mut self, c: Context<ConstraintCTokenMintAuthority>) -> ParseResult<()> {
        if self.ctoken_mint_authority.is_some() {
            return Err(ParseError::new(c.span(), "mint::authority already provided"));
        }
        self.ctoken_mint_authority.replace(c);
        Ok(())
    }

    fn add_ctoken_mint_freeze_authority(&mut self, c: Context<ConstraintCTokenMintFreezeAuthority>) -> ParseResult<()> {
        if self.ctoken_mint_freeze_authority.is_some() {
            return Err(ParseError::new(c.span(), "mint::freeze_authority already provided"));
        }
        self.ctoken_mint_freeze_authority.replace(c);
        Ok(())
    }

    fn add_ctoken_mint_payer(&mut self, c: Context<ConstraintCTokenMintPayer>) -> ParseResult<()> {
        if self.ctoken_mint_payer.is_some() {
            return Err(ParseError::new(c.span(), "mint::payer already provided"));
        }
        self.ctoken_mint_payer.replace(c);
        Ok(())
    }

    fn add_ctoken_mint_decimals(&mut self, c: Context<ConstraintCTokenMintDecimals>) -> ParseResult<()> {
        if self.ctoken_mint_decimals.is_some() {
            return Err(ParseError::new(c.span(), "mint::decimals already provided"));
        }
        self.ctoken_mint_decimals.replace(c);
        Ok(())
    }

    fn add_ctoken_mint_signer(&mut self, c: Context<ConstraintCTokenMintSigner>) -> ParseResult<()> {
        if self.ctoken_mint_signer.is_some() {
            return Err(ParseError::new(c.span(), "mint::mint_signer already provided"));
        }
        self.ctoken_mint_signer.replace(c);
        Ok(())
    }

    fn add_ctoken_mint_signer_seeds(&mut self, c: Context<ConstraintCTokenMintSignerSeeds>) -> ParseResult<()> {
        if self.ctoken_mint_signer_seeds.is_some() {
            return Err(ParseError::new(c.span(), "mint::mint_signer_seeds already provided"));
        }
        self.ctoken_mint_signer_seeds.replace(c);
        Ok(())
    }

    fn add_ctoken_mint_signer_bump(&mut self, c: Context<ConstraintCTokenMintSignerBump>) -> ParseResult<()> {
        if self.ctoken_mint_signer_bump.is_some() {
            return Err(ParseError::new(c.span(), "mint::mint_signer_bump already provided"));
        }
        self.ctoken_mint_signer_bump.replace(c);
        Ok(())
    }

    fn add_ctoken_mint_program_authority_seeds(&mut self, c: Context<ConstraintCTokenMintProgramAuthoritySeeds>) -> ParseResult<()> {
        if self.ctoken_mint_program_authority_seeds.is_some() {
            return Err(ParseError::new(c.span(), "mint::program_authority_seeds already provided"));
        }
        self.ctoken_mint_program_authority_seeds.replace(c);
        Ok(())
    }

    fn add_ctoken_mint_program_authority_bump(&mut self, c: Context<ConstraintCTokenMintProgramAuthorityBump>) -> ParseResult<()> {
        if self.ctoken_mint_program_authority_bump.is_some() {
            return Err(ParseError::new(c.span(), "mint::program_authority_bump already provided"));
        }
        self.ctoken_mint_program_authority_bump.replace(c);
        Ok(())
    }

    fn add_ctoken_mint_address_tree_info(&mut self, c: Context<ConstraintCTokenMintAddressTreeInfo>) -> ParseResult<()> {
        if self.ctoken_mint_address_tree_info.is_some() {
            return Err(ParseError::new(c.span(), "mint::address_tree_info already provided"));
        }
        self.ctoken_mint_address_tree_info.replace(c);
        Ok(())
    }

    fn add_ctoken_mint_proof(&mut self, c: Context<ConstraintCTokenMintProof>) -> ParseResult<()> {
        if self.ctoken_mint_proof.is_some() {
            return Err(ParseError::new(c.span(), "mint::proof already provided"));
        }
        self.ctoken_mint_proof.replace(c);
        Ok(())
    }

    fn add_ctoken_mint_output_state_tree_index(&mut self, c: Context<ConstraintCTokenMintOutputStateTreeIndex>) -> ParseResult<()> {
        if self.ctoken_mint_output_state_tree_index.is_some() {
            return Err(ParseError::new(c.span(), "mint::output_state_tree_index already provided"));
        }
        self.ctoken_mint_output_state_tree_index.replace(c);
        Ok(())
    }

    fn add_metadata_name(&mut self, c: Context<ConstraintMetadataName>) -> ParseResult<()> {
        if self.metadata_name.is_some() {
            return Err(ParseError::new(c.span(), "metadata::name already provided"));
        }
        self.metadata_name.replace(c);
        Ok(())
    }

    fn add_metadata_symbol(&mut self, c: Context<ConstraintMetadataSymbol>) -> ParseResult<()> {
        if self.metadata_symbol.is_some() {
            return Err(ParseError::new(c.span(), "metadata::symbol already provided"));
        }
        self.metadata_symbol.replace(c);
        Ok(())
    }

    fn add_metadata_uri(&mut self, c: Context<ConstraintMetadataUri>) -> ParseResult<()> {
        if self.metadata_uri.is_some() {
            return Err(ParseError::new(c.span(), "metadata::uri already provided"));
        }
        self.metadata_uri.replace(c);
        Ok(())
    }

    fn add_metadata_update_authority(&mut self, c: Context<ConstraintMetadataUpdateAuthority>) -> ParseResult<()> {
        if self.metadata_update_authority.is_some() {
            return Err(ParseError::new(c.span(), "metadata::update_authority already provided"));
        }
        self.metadata_update_authority.replace(c);
        Ok(())
    }

    fn add_metadata_additional(&mut self, c: Context<ConstraintMetadataAdditional>) -> ParseResult<()> {
        if self.metadata_additional.is_some() {
            return Err(ParseError::new(c.span(), "metadata::additional already provided"));
        }
        self.metadata_additional.replace(c);
        Ok(())
    }

    fn add_cpda_authority(&mut self, c: Context<ConstraintCPDAAuthority>) -> ParseResult<()> {
        if self.cpda_authority.is_some() {
            return Err(ParseError::new(c.span(), "cpda authority already provided"));
        }
        self.cpda_authority.replace(c);
        Ok(())
    }

    fn add_cpda_address_tree_info(&mut self, c: Context<ConstraintCPDAAddressTreeInfo>) -> ParseResult<()> {
        if self.cpda_address_tree_info.is_some() {
            return Err(ParseError::new(c.span(), "cpda address_tree_info already provided"));
        }
        self.cpda_address_tree_info.replace(c);
        Ok(())
    }

    fn add_cpda_proof(&mut self, c: Context<ConstraintCPDAProof>) -> ParseResult<()> {
        if self.cpda_proof.is_some() {
            return Err(ParseError::new(c.span(), "cpda proof already provided"));
        }
        self.cpda_proof.replace(c);
        Ok(())
    }

    fn add_cpda_output_state_tree_index(&mut self, c: Context<ConstraintCPDAOutputStateTreeIndex>) -> ParseResult<()> {
        if self.cpda_output_state_tree_index.is_some() {
            return Err(ParseError::new(c.span(), "cpda output_state_tree_index already provided"));
        }
        self.cpda_output_state_tree_index.replace(c);
        Ok(())
    }

    fn add_cpda_compress_on_init(&mut self, c: Context<ConstraintCPDACompressOnInit>) -> ParseResult<()> {
        if self.cpda_compress_on_init.is_some() {
            return Err(ParseError::new(c.span(), "compress_on_init already provided"));
        }
        self.cpda_compress_on_init.replace(c);
        Ok(())
    }

    fn add_cctoken(&mut self, c: Context<ConstraintCCToken>) -> ParseResult<()> {
        if self.cctoken.is_some() {
            return Err(ParseError::new(c.span(), "cctoken already provided"));
        }
        self.cctoken.replace(c);
        Ok(())
    }

    fn add_mut(&mut self, c: Context<ConstraintMut>) -> ParseResult<()> {
        if self.mutable.is_some() {
            return Err(ParseError::new(c.span(), "mut already provided"));
        }
        self.mutable.replace(c);
        Ok(())
    }

    fn add_signer(&mut self, c: Context<ConstraintSigner>) -> ParseResult<()> {
        if self.signer.is_some() {
            return Err(ParseError::new(c.span(), "signer already provided"));
        }
        self.signer.replace(c);
        Ok(())
    }

    fn add_has_one(&mut self, c: Context<ConstraintHasOne>) -> ParseResult<()> {
        if self
            .has_one
            .iter()
            .filter(|item| item.join_target == c.join_target)
            .count()
            > 0
        {
            return Err(ParseError::new(c.span(), "has_one target already provided"));
        }
        self.has_one.push(c);
        Ok(())
    }

    fn add_raw(&mut self, c: Context<ConstraintRaw>) -> ParseResult<()> {
        self.raw.push(c);
        Ok(())
    }

    fn add_owner(&mut self, c: Context<ConstraintOwner>) -> ParseResult<()> {
        if self.owner.is_some() {
            return Err(ParseError::new(c.span(), "owner already provided"));
        }
        self.owner.replace(c);
        Ok(())
    }

    fn add_rent_exempt(&mut self, c: Context<ConstraintRentExempt>) -> ParseResult<()> {
        if self.rent_exempt.is_some() {
            return Err(ParseError::new(c.span(), "rent already provided"));
        }
        self.rent_exempt.replace(c);
        Ok(())
    }

    fn add_seeds(&mut self, c: Context<ConstraintSeeds>) -> ParseResult<()> {
        if self.seeds.is_some() {
            return Err(ParseError::new(c.span(), "seeds already provided"));
        }
        self.seeds.replace(c);
        Ok(())
    }

    fn add_executable(&mut self, c: Context<ConstraintExecutable>) -> ParseResult<()> {
        if self.executable.is_some() {
            return Err(ParseError::new(c.span(), "executable already provided"));
        }
        self.executable.replace(c);
        Ok(())
    }

    fn add_payer(&mut self, c: Context<ConstraintPayer>) -> ParseResult<()> {
        if self.init.is_none() {
            return Err(ParseError::new(
                c.span(),
                "init must be provided before payer",
            ));
        }
        if self.payer.is_some() {
            return Err(ParseError::new(c.span(), "payer already provided"));
        }
        self.payer.replace(c);
        Ok(())
    }

    fn add_space(&mut self, c: Context<ConstraintSpace>) -> ParseResult<()> {
        if self.init.is_none() {
            return Err(ParseError::new(
                c.span(),
                "init must be provided before space",
            ));
        }
        if self.space.is_some() {
            return Err(ParseError::new(c.span(), "space already provided"));
        }
        self.space.replace(c);
        Ok(())
    }

    // extensions

    fn add_extension_group_pointer_authority(
        &mut self,
        c: Context<ConstraintExtensionAuthority>,
    ) -> ParseResult<()> {
        if self.extension_group_pointer_authority.is_some() {
            return Err(ParseError::new(
                c.span(),
                "extension group pointer authority already provided",
            ));
        }
        self.extension_group_pointer_authority.replace(c);
        Ok(())
    }

    fn add_extension_group_pointer_group_address(
        &mut self,
        c: Context<ConstraintExtensionGroupPointerGroupAddress>,
    ) -> ParseResult<()> {
        if self.extension_group_pointer_group_address.is_some() {
            return Err(ParseError::new(
                c.span(),
                "extension group pointer group address already provided",
            ));
        }
        self.extension_group_pointer_group_address.replace(c);
        Ok(())
    }

    fn add_extension_group_member_pointer_authority(
        &mut self,
        c: Context<ConstraintExtensionAuthority>,
    ) -> ParseResult<()> {
        if self.extension_group_member_pointer_authority.is_some() {
            return Err(ParseError::new(
                c.span(),
                "extension group member pointer authority already provided",
            ));
        }
        self.extension_group_member_pointer_authority.replace(c);
        Ok(())
    }

    fn add_extension_group_member_pointer_member_address(
        &mut self,
        c: Context<ConstraintExtensionGroupMemberPointerMemberAddress>,
    ) -> ParseResult<()> {
        if self.extension_group_member_pointer_member_address.is_some() {
            return Err(ParseError::new(
                c.span(),
                "extension group member pointer member address already provided",
            ));
        }
        self.extension_group_member_pointer_member_address
            .replace(c);
        Ok(())
    }

    fn add_extension_metadata_pointer_authority(
        &mut self,
        c: Context<ConstraintExtensionAuthority>,
    ) -> ParseResult<()> {
        if self.extension_metadata_pointer_authority.is_some() {
            return Err(ParseError::new(
                c.span(),
                "extension metadata pointer authority already provided",
            ));
        }
        self.extension_metadata_pointer_authority.replace(c);
        Ok(())
    }

    fn add_extension_metadata_pointer_metadata_address(
        &mut self,
        c: Context<ConstraintExtensionMetadataPointerMetadataAddress>,
    ) -> ParseResult<()> {
        if self.extension_metadata_pointer_metadata_address.is_some() {
            return Err(ParseError::new(
                c.span(),
                "extension metadata pointer metadata address already provided",
            ));
        }
        self.extension_metadata_pointer_metadata_address.replace(c);
        Ok(())
    }

    fn add_extension_close_authority(
        &mut self,
        c: Context<ConstraintExtensionAuthority>,
    ) -> ParseResult<()> {
        if self.extension_close_authority.is_some() {
            return Err(ParseError::new(
                c.span(),
                "extension close authority already provided",
            ));
        }
        self.extension_close_authority.replace(c);
        Ok(())
    }

    fn add_extension_authority(
        &mut self,
        c: Context<ConstraintExtensionAuthority>,
    ) -> ParseResult<()> {
        if self.extension_transfer_hook_authority.is_some() {
            return Err(ParseError::new(
                c.span(),
                "extension transfer hook authority already provided",
            ));
        }
        self.extension_transfer_hook_authority.replace(c);
        Ok(())
    }

    fn add_extension_transfer_hook_program_id(
        &mut self,
        c: Context<ConstraintExtensionTokenHookProgramId>,
    ) -> ParseResult<()> {
        if self.extension_transfer_hook_program_id.is_some() {
            return Err(ParseError::new(
                c.span(),
                "extension transfer hook program id already provided",
            ));
        }
        self.extension_transfer_hook_program_id.replace(c);
        Ok(())
    }

    fn add_extension_permanent_delegate(
        &mut self,
        c: Context<ConstraintExtensionPermanentDelegate>,
    ) -> ParseResult<()> {
        if self.extension_permanent_delegate.is_some() {
            return Err(ParseError::new(
                c.span(),
                "extension permanent delegate already provided",
            ));
        }
        self.extension_permanent_delegate.replace(c);
        Ok(())
    }
}

/// Validates that metadata field expressions have appropriate size limits at compile time.
/// This function checks if the expression is a string literal and validates its length.
fn validate_metadata_size_limit(
    expr: &Expr,
    field_name: &str,
    max_bytes: usize,
    span: proc_macro2::Span,
) -> ParseResult<()> {
    match expr {
        // Check string literals
        Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(lit_str), .. }) => {
            let str_value = lit_str.value();
            let byte_len = str_value.len();
            
            if byte_len > max_bytes {
                return Err(ParseError::new(
                    span,
                    format!(
                        "CMint metadata {} exceeds size limit: {} bytes (max: {} bytes).",
                        field_name, byte_len, max_bytes
                    )
                ));
            }
        }
        // Check byte string literals
        Expr::Lit(syn::ExprLit { lit: syn::Lit::ByteStr(lit_bytes), .. }) => {
            let byte_len = lit_bytes.value().len();
            
            if byte_len > max_bytes {
                return Err(ParseError::new(
                    span,
                    format!(
                        "CMint metadata {} exceeds size limit: {} bytes (max: {} bytes).", field_name, byte_len, max_bytes
                    )
                ));
            }
        }
        // For other expressions (variables, function calls, etc.), we can't validate at compile time
        // The runtime validation will be handled in the generated code
        _ => {
            // No compile-time validation possible for dynamic expressions
            // Runtime validation will be added in the generated code
        }
    }
    
    Ok(())
}
