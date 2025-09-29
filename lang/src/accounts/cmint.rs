use crate::{
    AccountInfo, Accounts, AccountsFinalize, Key, Result, ToAccountInfo, ToAccountInfos,
    ToAccountMetas,
};
use solana_program::instruction::AccountMeta;
use solana_program::pubkey::Pubkey;
use std::cell::RefCell;
use std::collections::BTreeSet;

#[derive(Clone)]
pub struct MintAction<'info> {
    pub recipient: AccountInfo<'info>,
    pub amount: u64,
}

/// CMint is a compressed mint account wrapper.
/// It queues mint actions to be executed in a single batched invoke at finalize.
pub struct CMint<'info> {
    info: AccountInfo<'info>,
    actions: RefCell<Vec<MintAction<'info>>>,
    // Constraints from account macro
    pub authority: Option<Pubkey>,
    pub decimals: Option<u8>,
    pub mint_signer: Option<AccountInfo<'info>>,
    pub mint_signer_seeds: Option<Vec<Vec<u8>>>,
    pub mint_signer_bump: Option<u8>,
    pub program_authority_seeds: Option<Vec<Vec<u8>>>,
    pub program_authority_bump: Option<u8>,
}

impl<'info> CMint<'info> {
    pub fn mint_to(&self, recipient: &AccountInfo<'info>, amount: u64) -> crate::Result<()> {
        self.actions.borrow_mut().push(MintAction {
            recipient: recipient.clone(),
            amount,
        });
        Ok(())
    }

    pub fn take_actions(&self) -> Vec<MintAction<'info>> {
        let mut b = self.actions.borrow_mut();
        let actions = b.clone();
        b.clear();
        actions
    }
}

impl<'info> ToAccountInfo<'info> for CMint<'info> {
    fn to_account_info(&self) -> AccountInfo<'info> {
        self.info.clone()
    }
}

impl<'info> ToAccountInfos<'info> for CMint<'info> {
    fn to_account_infos(&self) -> Vec<AccountInfo<'info>> {
        vec![self.info.clone()]
    }
}

impl<'info> ToAccountMetas for CMint<'info> {
    fn to_account_metas(&self, is_signer: Option<bool>) -> Vec<AccountMeta> {
        let signer = is_signer.unwrap_or(false);
        vec![AccountMeta::new(self.key(), signer)]
    }
}

impl<'info> Key for CMint<'info> {
    fn key(&self) -> Pubkey {
        *self.info.key
    }
}

impl<'info, T> Accounts<'info, T> for CMint<'info> {
    fn try_accounts(
        _program_id: &Pubkey,
        accounts: &mut &'info [AccountInfo<'info>],
        _ix_data: &[u8],
        _bumps: &mut T,
        _reallocs: &mut BTreeSet<Pubkey>,
    ) -> Result<Self> {
        if accounts.is_empty() {
            return Err(crate::error::ErrorCode::AccountNotEnoughKeys.into());
        }
        let account = &accounts[0];
        *accounts = &accounts[1..];
        Ok(CMint {
            info: account.clone(),
            actions: RefCell::new(Vec::new()),
            authority: None,
            decimals: None,
            mint_signer: None,
            mint_signer_seeds: None,
            mint_signer_bump: None,
            program_authority_seeds: None,
            program_authority_bump: None,
        })
    }
}

// Note: the outer Accounts struct finalize will consume actions and perform the batched CPI.
impl<'info, B> AccountsFinalize<'info, B> for CMint<'info> {}
