use log::*;
use std::collections::HashSet;

#[derive(Debug)]
pub(crate) struct AccountsSelector {
    pub accounts: HashSet<Vec<u8>>,
    pub owners: HashSet<Vec<u8>>,
    pub select_all_accounts: bool,
    pub token_account_closure: bool,
    pub solana_program: Vec<u8>,
}

const SOLANA_PROGRAM_LITERAL: &str = "11111111111111111111111111111111";

impl AccountsSelector {
    pub fn default() -> Self {
        AccountsSelector {
            accounts: HashSet::default(),
            owners: HashSet::default(),
            select_all_accounts: true,
            token_account_closure: false,
            solana_program: bs58::decode(SOLANA_PROGRAM_LITERAL.to_string())
                .into_vec()
                .unwrap(),
        }
    }

    pub fn new(accounts: &[String], owners: &[String], token_account_closure: bool) -> Self {
        info!(
            "Creating AccountsSelector from accounts: {:?}, owners: {:?}",
            accounts, owners
        );

        let select_all_accounts = accounts.iter().any(|key| key == "*");
        let solana_program = bs58::decode(SOLANA_PROGRAM_LITERAL.to_string())
            .into_vec()
            .unwrap();
        if select_all_accounts {
            return AccountsSelector {
                accounts: HashSet::default(),
                owners: HashSet::default(),
                select_all_accounts,
                token_account_closure,
                solana_program,
            };
        }
        let accounts = accounts
            .iter()
            .map(|key| bs58::decode(key).into_vec().unwrap())
            .collect();
        let owners = owners
            .iter()
            .map(|key| bs58::decode(key).into_vec().unwrap())
            .collect();
        AccountsSelector {
            accounts,
            owners,
            select_all_accounts,
            token_account_closure,
            solana_program,
        }
    }

    pub fn is_account_selected(&self, account: &[u8], owner: &[u8], lamports: u64) -> bool {
        self.select_all_accounts
            || self.accounts.contains(account)
            || self.owners.contains(owner)
            || (self.token_account_closure && owner == self.solana_program && lamports == 0)
    }

    /// Check if any account is of interested at all
    pub fn is_enabled(&self) -> bool {
        self.select_all_accounts || !self.accounts.is_empty() || !self.owners.is_empty()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn test_create_accounts_selector() {
        AccountsSelector::new(
            &["9xQeWvG816bUx9EPjHmaT23yvVM2ZWbrrpZb9PusVFin".to_string()],
            &[],
            false,
        );

        AccountsSelector::new(
            &[],
            &["9xQeWvG816bUx9EPjHmaT23yvVM2ZWbrrpZb9PusVFin".to_string()],
            false,
        );
    }
}
