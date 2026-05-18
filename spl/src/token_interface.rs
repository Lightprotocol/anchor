pub use crate::token_2022::*;
#[cfg(feature = "token_2022_extensions")]
pub use crate::token_2022_extensions::*;
use {
    anchor_lang::{
        __private::bytemuck::Pod,
        solana_program::{program_pack::Pack, pubkey::Pubkey},
    },
    spl_token_2022::extension::{
        BaseStateWithExtensions, Extension, ExtensionType, StateWithExtensions,
    },
    std::ops::Deref,
};

pub const COMPRESSED_TOKEN_ID: Pubkey = Pubkey::new_from_array([
    9, 21, 163, 87, 35, 121, 78, 143, 182, 93, 7, 91, 107, 114, 105, 156, 56, 221, 2, 229, 148,
    139, 117, 176, 229, 160, 65, 142, 128, 151, 91, 68,
]);

static IDS: [Pubkey; 3] = [
    spl_token_interface::ID,
    spl_token_2022_interface::ID,
    COMPRESSED_TOKEN_ID,
];

macro_rules! impl_empty_idl_build {
    ($ty:ty) => {
        impl $ty {
            pub const DISCRIMINATOR: &'static [u8] = &[];

            pub fn create_type() -> Option<anchor_lang_idl::types::IdlTypeDef> {
                None
            }

            pub fn insert_types(
                _types: &mut std::collections::BTreeMap<String, anchor_lang_idl::types::IdlTypeDef>,
            ) {
            }

            pub fn get_full_path() -> String {
                std::any::type_name::<Self>().into()
            }
        }
    };
}

#[derive(Clone, Debug, Default, PartialEq, Copy)]
pub struct TokenAccount(spl_token_2022::state::Account);

impl_empty_idl_build!(TokenAccount);

impl anchor_lang::AccountDeserialize for TokenAccount {
    fn try_deserialize_unchecked(buf: &mut &[u8]) -> anchor_lang::Result<Self> {
        spl_token_2022::extension::StateWithExtensions::<spl_token_2022::state::Account>::unpack(
            buf,
        )
        .map(|t| TokenAccount(t.base))
        .map_err(Into::into)
    }
}

impl anchor_lang::AccountSerialize for TokenAccount {}

impl anchor_lang::Owners for TokenAccount {
    fn owners() -> &'static [Pubkey] {
        &IDS
    }
}

impl Deref for TokenAccount {
    type Target = spl_token_2022::state::Account;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Clone, Debug, Default, PartialEq, Copy)]
pub struct Mint(spl_token_2022::state::Mint);

impl_empty_idl_build!(Mint);

impl anchor_lang::AccountDeserialize for Mint {
    fn try_deserialize_unchecked(buf: &mut &[u8]) -> anchor_lang::Result<Self> {
        // If >= 166 bytes, validate account type byte is Mint (1)
        if buf.len() >= 166 && buf[165] != 1 {
            return Err(anchor_lang::error::ErrorCode::AccountDidNotDeserialize.into());
        }

        // Deserialize base mint only (82 bytes)
        let base_slice = &buf[..spl_token_2022::state::Mint::LEN];
        spl_token_2022::extension::StateWithExtensions::<spl_token_2022::state::Mint>::unpack(
            base_slice,
        )
        .map(|t| Mint(t.base))
        .map_err(Into::into)
    }
}

impl anchor_lang::AccountSerialize for Mint {}

impl anchor_lang::Owners for Mint {
    fn owners() -> &'static [Pubkey] {
        &IDS
    }
}

impl Deref for Mint {
    type Target = spl_token_2022::state::Mint;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Clone)]
pub struct TokenInterface;

impl anchor_lang::Ids for TokenInterface {
    fn ids() -> &'static [Pubkey] {
        &IDS
    }
}

pub type ExtensionsVec = Vec<ExtensionType>;

pub fn find_mint_account_size(extensions: Option<&ExtensionsVec>) -> anchor_lang::Result<usize> {
    if let Some(extensions) = extensions {
        Ok(ExtensionType::try_calculate_account_len::<
            spl_token_2022::state::Mint,
        >(extensions)?)
    } else {
        Ok(spl_token_2022::state::Mint::LEN)
    }
}

pub fn get_mint_extension_data<T: Extension + Pod>(
    account: &anchor_lang::solana_program::account_info::AccountInfo,
) -> anchor_lang::Result<T> {
    let mint_data = account.data.borrow();
    let mint_with_extension =
        StateWithExtensions::<spl_token_2022::state::Mint>::unpack(&mint_data)?;
    let extension_data = *mint_with_extension.get_extension::<T>()?;
    Ok(extension_data)
}
