mod price_receiver;

use crate::price_receiver::*;

use near_contract_standards::fungible_token::core_impl::ext_fungible_token;
use near_contract_standards::fungible_token::metadata::{
    FungibleTokenMetadata, FungibleTokenMetadataProvider,
};
use near_contract_standards::fungible_token::receiver::FungibleTokenReceiver;
use near_contract_standards::fungible_token::FungibleToken;
use near_sdk::borsh::{self, BorshDeserialize, BorshSerialize};
use near_sdk::collections::LazyOption;
use near_sdk::json_types::{ValidAccountId, U128};
use near_sdk::serde::{Deserialize, Serialize};
use near_sdk::{
    assert_one_yocto, env, ext_contract, is_promise_success, log, near_bindgen, AccountId, Balance,
    BorshStorageKey, Gas, PanicOnDefault, Promise, PromiseOrValue, Timestamp,
};

near_sdk::setup_alloc!();

const OWNER_ID: &str = "dreamproject.near";
const NO_DEPOSIT: Balance = 0;
const ONE_YOCTO: Balance = 1;

const TGAS: Gas = 1_000_000_000_000;
const GAS_FOR_FT_TRANSFER: Gas = 10 * TGAS;
const GAS_FOR_AFTER_FT_TRANSFER: Gas = 10 * TGAS;

pub type TokenAccountId = AccountId;

#[derive(BorshSerialize, BorshStorageKey)]
enum StorageKey {
    Ft,
    FtMeta,
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Copy, Clone)]
#[serde(crate = "near_sdk::serde")]
pub enum Status {
    Locked,
    Unlocking {
        #[serde(with = "u64_dec_format")]
        initiated_timestamp: Timestamp,
    },
    Unlocked,
}

#[ext_contract(ext_self)]
pub trait ExtSelf {
    fn after_ft_transfer(&mut self, account_id: AccountId, balance: U128) -> bool;
}

pub trait ExtSelf {
    fn after_ft_transfer(&mut self, account_id: AccountId, balance: U128) -> bool;
}

#[near_bindgen]
#[derive(BorshDeserialize, BorshSerialize, PanicOnDefault, Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct Contract {
    #[serde(skip)]
    pub ft: FungibleToken,
    #[serde(skip)]
    pub meta: LazyOption<FungibleTokenMetadata>,
    pub backup_trigger_account_id: Option<AccountId>,
    pub price_oracle_account_id: AccountId,
    pub asset_id: AssetId,
    pub minimum_unlock_price: Price,
    pub locked_token_account_id: TokenAccountId,
    pub status: Status,
}

near_contract_standards::impl_fungible_token_core!(Contract, ft, on_tokens_burned);
near_contract_standards::impl_fungible_token_storage!(Contract, ft, on_account_closed);

#[near_bindgen]
impl FungibleTokenMetadataProvider for Contract {
    fn ft_metadata(&self) -> FungibleTokenMetadata {
        self.meta.get().unwrap()
    }
}

#[near_bindgen]
impl FungibleTokenReceiver for Contract {
    #[allow(unused_variables)]
    fn ft_on_transfer(
        &mut self,
        sender_id: ValidAccountId,
        amount: U128,
        msg: String,
    ) -> PromiseOrValue<U128> {
        assert_eq!(
            &env::predecessor_account_id(),
            &self.locked_token_account_id
        );
        assert!(matches!(self.status, Status::Locked));
        self.ft.internal_deposit(sender_id.as_ref(), amount.0);
        return PromiseOrValue::Value(U128(0));
    }
}

#[near_bindgen]
impl ExtSelf for Contract {
    #[private]
    fn after_ft_transfer(&mut self, account_id: AccountId, balance: U128) -> bool {
        let promise_success = is_promise_success();
        if promise_success {
            if let Some(balance) = self.ft.accounts.get(&account_id) {
                if balance == 0 {
                    self.ft.accounts.remove(&account_id);
                    Promise::new(account_id).transfer(self.storage_balance_bounds().min.0);
                }
            }
        } else {
            log!("Failed to transfer {} to account {}", account_id, balance.0);
            self.ft.internal_deposit(&account_id, balance.into());
        }
        promise_success
    }
}

#[near_bindgen]
impl Contract {
    #[init]
    pub fn new(
        locked_token_account_id: ValidAccountId,
        meta: FungibleTokenMetadata,
        backup_trigger_account_id: Option<ValidAccountId>,
        price_oracle_account_id: ValidAccountId,
        asset_id: AssetId,
        minimum_unlock_price: Price,
    ) -> Self {
        Self {
            ft: FungibleToken::new(StorageKey::Ft),
            meta: LazyOption::new(StorageKey::FtMeta, Some(&meta)),
            backup_trigger_account_id: backup_trigger_account_id.map(|a| a.into()),
            locked_token_account_id: locked_token_account_id.into(),
            status: Status::Locked,
            price_oracle_account_id: price_oracle_account_id.into(),
            asset_id,
            minimum_unlock_price,
        }
    }

    pub fn get_info(self) -> Self {
        self
    }

    #[payable]
    pub fn unlock(&mut self) {
        assert_one_yocto();
        assert_eq!(
            &Some(env::predecessor_account_id()),
            &self.backup_trigger_account_id
        );
        assert!(!matches!(self.status, Status::Unlocked));
        self.status = Status::Unlocked;
    }

    #[payable]
    pub fn unwrap(&mut self) -> Promise {
        assert_one_yocto();
        assert!(matches!(self.status, Status::Unlocked));
        let account_id = env::predecessor_account_id();
        let balance = self.ft.accounts.get(&account_id).unwrap_or(0);
        self.ft.internal_withdraw(&account_id, balance);
        ext_fungible_token::ft_transfer(
            account_id.clone(),
            U128(balance),
            Some(format!("Unwrapping {} tokens", env::current_account_id())),
            &self.locked_token_account_id,
            ONE_YOCTO,
            GAS_FOR_FT_TRANSFER,
        )
        .then(ext_self::after_ft_transfer(
            account_id,
            U128(balance),
            &env::current_account_id(),
            NO_DEPOSIT,
            GAS_FOR_AFTER_FT_TRANSFER,
        ))
    }

    pub fn update_meta (&mut self, meta: FungibleTokenMetadata){
        assert_eq!(env::predecessor_account_id(), OWNER_ID, "No Access");
        self.meta = LazyOption::new(StorageKey::FtMeta, Some(&meta));
    }

    pub fn get_status (&self) -> Status { self.status }

    fn on_account_closed(&mut self, account_id: AccountId, balance: Balance) {
        log!("Closed @{} with {}", account_id, balance);
    }

    fn on_tokens_burned(&mut self, account_id: AccountId, amount: Balance) {
        log!("Account @{} burned {}", account_id, amount);
    }
}
