use near_contract_standards::fungible_token::metadata::FungibleTokenMetadata;
use near_sdk::{
    AccountId, Balance, BorshStorageKey, env, ext_contract, Gas, log, near_bindgen, PanicOnDefault, Promise,
};
use near_sdk::borsh::{self, BorshDeserialize, BorshSerialize};
use near_sdk::collections::{LookupMap, UnorderedMap};
use near_sdk::env::STORAGE_PRICE_PER_BYTE;
use near_sdk::json_types::{U128, ValidAccountId};
use near_sdk::serde::{Deserialize, Serialize};
use near_sdk::serde_json;

mod migrate;

near_sdk::setup_alloc!();

const FT_WASM_CODE: &[u8] = include_bytes!("../../token/res/locked_ft.wasm");

const EXTRA_BYTES: usize = 10000;
const GAS: Gas = 50_000_000_000_000;
const GAS_FT_METADATA_READ: Gas = 25_000_000_000_000;
const GAS_FT_METADATA_WRITE: Gas = 25_000_000_000_000;
const NO_DEPOSIT: Balance = 0;

type TokenId = String;

pub fn is_valid_symbol(token_id: &TokenId) -> bool {
    for c in token_id.as_bytes() {
        match c {
            b'0'..=b'9' | b'a'..=b'z' | b'_' | b'-' => (),
            _ => return false,
        }
    }
    true
}

#[ext_contract(ext_ft)]
pub trait ExtFT {
    /// Get FT metadata.
    fn ft_metadata(&self) -> FungibleTokenMetadata;
}

#[ext_contract(ext_self)]
pub trait ExtContract {
    /// Save FT metadata
    fn on_ft_metadata(
        &mut self,
        token_id: AccountId,
        asset_id: AccountId,
    );
}

#[derive(BorshSerialize, BorshStorageKey)]
enum StorageKey {
    Tokens,
    StorageDeposits,
    WhitelistedTokens,
    WhitelistedTokensV1,
}

#[near_bindgen]
#[derive(BorshDeserialize, BorshSerialize, PanicOnDefault)]
pub struct TokenFactory {
    pub tokens: UnorderedMap<TokenId, TokenArgs>,
    pub storage_deposits: LookupMap<AccountId, Balance>,
    pub storage_balance_cost: Balance,
    pub whitelisted_tokens: UnorderedMap<AccountId, WhitelistedToken>,
}

#[derive(BorshDeserialize, BorshSerialize, PanicOnDefault, Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct WhitelistedToken {
    pub asset_id: String,
    pub metadata: FungibleTokenMetadata
}

#[derive(Serialize, Deserialize, BorshDeserialize, BorshSerialize)]
#[serde(crate = "near_sdk::serde")]
pub struct InputTokenArgs {
    token_id: ValidAccountId,
    target_price: U128,
    backup_trigger_account_id: Option<AccountId>,
    price_oracle_account_id: AccountId,
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Clone, Copy)]
#[serde(crate = "near_sdk::serde")]
pub struct Price {
    #[serde(with = "u128_dec_format")]
    multiplier: Balance,
    decimals: u8,
}

pub type AssetId = String;
pub type TokenAccountId = AccountId;

#[derive(BorshDeserialize, BorshSerialize, PanicOnDefault, Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct TokenArgs {
    pub locked_token_account_id: TokenAccountId,
    pub meta: FungibleTokenMetadata,
    pub backup_trigger_account_id: Option<AccountId>,
    pub price_oracle_account_id: AccountId,
    pub asset_id: AssetId,
    pub minimum_unlock_price: Price,
}

#[near_bindgen]
impl TokenFactory {
    #[init]
    pub fn new() -> Self {
        let mut storage_deposits = LookupMap::new(StorageKey::StorageDeposits);

        let initial_storage_usage = env::storage_usage();
        let tmp_account_id = "a".repeat(64);
        storage_deposits.insert(&tmp_account_id, &0);
        let storage_balance_cost =
            Balance::from(env::storage_usage() - initial_storage_usage) * STORAGE_PRICE_PER_BYTE;
        storage_deposits.remove(&tmp_account_id);

        Self {
            tokens: UnorderedMap::new(StorageKey::Tokens),
            storage_deposits,
            storage_balance_cost,
            whitelisted_tokens: UnorderedMap::new(StorageKey::WhitelistedTokens),
        }
    }

    pub fn on_ft_metadata(
        &mut self,
        #[callback] ft_metadata: FungibleTokenMetadata,
        token_id: AccountId,
        asset_id: AssetId) {
        assert!(
            is_valid_symbol(&ft_metadata.symbol.to_ascii_lowercase()),
            "Invalid Token symbol"
        );

        self.whitelisted_tokens.insert(
            &(token_id.into()),
            &WhitelistedToken {
                asset_id: asset_id.into(),
                metadata: ft_metadata,
            },
        );
    }

    #[private]
    pub fn whitelist_token(
        &mut self,
        token_id: ValidAccountId,
        asset_id: ValidAccountId
    ) -> Promise {
            ext_ft::ft_metadata(
                &token_id,
                NO_DEPOSIT,
                GAS_FT_METADATA_READ,
            ).then(ext_self::on_ft_metadata(
                token_id.into(),
                asset_id.into(),
                &env::current_account_id(),
                NO_DEPOSIT,
                GAS_FT_METADATA_WRITE,
            ))
    }

    fn get_min_attached_balance(&self, args: &TokenArgs) -> u128 {
        ((FT_WASM_CODE.len() + EXTRA_BYTES + args.try_to_vec().unwrap().len() * 2) as Balance
            * STORAGE_PRICE_PER_BYTE)
            .into()
    }

    #[payable]
    pub fn storage_deposit(&mut self) {
        let account_id = env::predecessor_account_id();
        let deposit = env::attached_deposit();
        if let Some(previous_balance) = self.storage_deposits.get(&account_id) {
            self.storage_deposits
                .insert(&account_id, &(previous_balance + deposit));
        } else {
            assert!(deposit >= self.storage_balance_cost, "Deposit is too low");
            self.storage_deposits
                .insert(&account_id, &(deposit - self.storage_balance_cost));
        }
    }

    pub fn get_number_of_tokens(&self) -> u64 {
        self.tokens.len()
    }

    pub fn get_tokens(&self, from_index: u64, limit: u64) -> Vec<TokenArgs> {
        let tokens = self.tokens.values_as_vector();
        (from_index..std::cmp::min(from_index + limit, tokens.len()))
            .filter_map(|index| tokens.get(index))
            .collect()
    }

    pub fn get_whitelisted_token_tickers(&self, from_index: u64, limit: u64) -> Vec<TokenAccountId> {
        let token_ids = self.whitelisted_tokens.keys_as_vector();
        (from_index..std::cmp::min(from_index + limit, token_ids.len()))
            .filter_map(|token_id| token_ids.get(token_id))
            .collect()
    }

    pub fn get_whitelisted_token(&self, token_id: TokenAccountId) -> WhitelistedToken {
        self.whitelisted_tokens.get(&token_id).expect("Token not found")
    }

    pub fn get_token(&self, token_id: TokenId) -> Option<TokenArgs> {
        self.tokens.get(&token_id)
    }

    #[private]
    pub fn update_token_icon(&mut self, token_id: TokenAccountId, icon: Option<String>) {
        let mut token = self.whitelisted_tokens.get(&token_id).expect("Token wasn't whitelisted");
        token.metadata.icon = icon;
        self.whitelisted_tokens.insert(&token_id, &token);
    }

    #[payable]
    pub fn create_token(&mut self, token_args: InputTokenArgs) -> Promise {
        if env::attached_deposit() > 0 {
            self.storage_deposit();
        }

        let whitelisted_token = self
            .whitelisted_tokens
            .get(&(token_args.token_id.clone().into()))
            .expect("Token wasn't whitelisted");
        let token_name = TokenFactory::format_title(whitelisted_token.metadata.symbol.clone());
        let token_decimals = whitelisted_token.metadata.decimals;

        assert!(token_decimals > 0 && token_name != "", "Missing token metadata");

        let mut metadata = whitelisted_token.metadata;

        let minimum_unlock_price = Price {
            multiplier: token_args.target_price.0,
            decimals: token_decimals + 4,
        };

        let target_price_short: u128 = token_args.target_price.0 / 10000;
        let target_price_remainder: u128 = token_args.target_price.0 % 10000;

        let price = if target_price_remainder > 0 {
            format!("{}.{}", target_price_short, target_price_remainder)
        } else {
            format!("{}", target_price_short)
        };
        assert!(token_args.target_price.0 > 0, "Wrong target price");

        metadata.name = format!("{} at ${}", token_name, price);
        metadata.symbol = format!("{}@{}", token_name, price);

        metadata.assert_valid();

        let token_id = format!(
            "{}-{}-{:04}",
            token_name, target_price_short, target_price_remainder
        )
        .to_ascii_lowercase();

        let token_account_id = format!("{}.{}", token_id, env::current_account_id());
        assert!(
            env::is_valid_account_id(token_account_id.as_bytes()),
            "Token Account ID is invalid"
        );

        let args: TokenArgs = TokenArgs {
            locked_token_account_id: token_args.token_id.clone().into(),
            meta: metadata,
            backup_trigger_account_id: token_args.backup_trigger_account_id.map(|a| a.into()),
            price_oracle_account_id: token_args.price_oracle_account_id.into(),
            asset_id: whitelisted_token.asset_id.clone(),
            minimum_unlock_price,
        };

        let account_id = env::predecessor_account_id();

        let required_balance = self.get_min_attached_balance(&args);
        let user_balance = self.storage_deposits.get(&account_id).unwrap_or(0);
        assert!(
            user_balance >= required_balance,
            "Not enough required balance"
        );
        self.storage_deposits
            .insert(&account_id, &(user_balance - required_balance));

        let initial_storage_usage = env::storage_usage();

        assert!(
            self.tokens.insert(&token_id, &args).is_none(),
            "Token ID {} is already taken",
            token_id
        );

        log!(
            "Creating token {} with asset {} at price {}",
            token_account_id,
            whitelisted_token.asset_id,
            price
        );

        let storage_balance_used =
            Balance::from(env::storage_usage() - initial_storage_usage) * STORAGE_PRICE_PER_BYTE;

        Promise::new(token_account_id)
            .create_account()
            .transfer(required_balance - storage_balance_used)
            .deploy_contract(FT_WASM_CODE.to_vec())
            .function_call(b"new".to_vec(), serde_json::to_vec(&args).unwrap(), 0, GAS)
    }

    fn format_title(s: String) -> String {
        s.chars().filter(|c| !c.is_whitespace()).collect()
    }
}

pub mod u64_dec_format {
    use near_sdk::serde::{Deserialize, Deserializer, Serializer};
    use near_sdk::serde::de;

    pub fn serialize<S>(num: &u64, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&num.to_string())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<u64, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(de::Error::custom)
    }
}

pub mod u128_dec_format {
    use near_sdk::serde::{Deserialize, Deserializer, Serializer};
    use near_sdk::serde::de;

    pub fn serialize<S>(num: &u128, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&num.to_string())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<u128, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(de::Error::custom)
    }
}
