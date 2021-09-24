use near_contract_standards::fungible_token::metadata::FungibleTokenMetadata;
use near_sdk::borsh::{self, BorshDeserialize, BorshSerialize};
use near_sdk::collections::{LookupMap, UnorderedMap};
use near_sdk::env::STORAGE_PRICE_PER_BYTE;
use near_sdk::json_types::{ValidAccountId, U128};
use near_sdk::serde::{Deserialize, Serialize};
use near_sdk::serde_json;
use near_sdk::{
    env, near_bindgen, AccountId, Balance, BorshStorageKey, Gas, PanicOnDefault, Promise
};

near_sdk::setup_alloc!();

const FT_WASM_CODE: &[u8] = include_bytes!("../../token/res/locked_ft.wasm");

const EXTRA_BYTES: usize = 10000;
const GAS: Gas = 50_000_000_000_000;
type TokenId = String;

pub fn is_valid_token_id(token_id: &TokenId) -> bool {
    for c in token_id.as_bytes() {
        match c {
            b'0'..=b'9' | b'a'..=b'z' | b'_' => (),
            _ => return false,
        }
    }
    true
}

#[derive(BorshSerialize, BorshStorageKey)]
enum StorageKey {
    Tokens,
    StorageDeposits,
    WhitelistedTokens
}

#[near_bindgen]
#[derive(BorshDeserialize, BorshSerialize, PanicOnDefault)]
pub struct TokenFactory {
    pub tokens: UnorderedMap<TokenId, TokenArgs>,
    pub storage_deposits: LookupMap<AccountId, Balance>,
    pub storage_balance_cost: Balance,
    pub whitelisted_tokens: UnorderedMap<AccountId, WhitelistedToken>
}

#[derive(BorshDeserialize, BorshSerialize, PanicOnDefault)]
pub struct WhitelistedToken {
    pub title: String,
    pub decimals: u8
}


#[derive(Serialize, Deserialize, BorshDeserialize, BorshSerialize)]
#[serde(crate = "near_sdk::serde")]
pub struct InputTokenArgs {
    token_id: ValidAccountId,
    target_price: U128,
    metadata: FungibleTokenMetadata,
    backup_trigger_account_id: Option<AccountId>,
    price_oracle_account_id: AccountId
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
    pub minimum_unlock_price: Price
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

    #[private]
    pub fn whitelist_token(&mut self, token_id: ValidAccountId, title: String, decimals: u8) {
        self.whitelisted_tokens.insert(&(token_id.into()),
                                       &WhitelistedToken{title, decimals});
    }

    fn get_min_attached_balance(&self, args: &TokenArgs) -> u128 {
        ((FT_WASM_CODE.len() + EXTRA_BYTES + args.try_to_vec().unwrap().len() * 2) as Balance
            * STORAGE_PRICE_PER_BYTE)
            .into()
    }

    /*
    pub fn get_required_deposit(&self, args: TokenArgs, account_id: ValidAccountId) -> U128 {
        let args_deposit = self.get_min_attached_balance(&args);
        if let Some(previous_balance) = self.storage_deposits.get(account_id.as_ref()) {
            args_deposit.saturating_sub(previous_balance).into()
        } else {
            (self.storage_balance_cost + args_deposit).into()
        }
    }*/

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

    pub fn get_token(&self, token_id: TokenId) -> Option<TokenArgs> {
        self.tokens.get(&token_id)
    }

    #[payable]
    pub fn create_token(&mut self, mut token_args: InputTokenArgs) -> Promise {
        if env::attached_deposit() > 0 {
            self.storage_deposit();
        }
        token_args.metadata.assert_valid();

        let whitelisted_token = self.whitelisted_tokens.get(&(token_args.token_id.clone().into())).expect("Token wasn't whitelisted");
        let token_name = whitelisted_token.title;
        let token_decimals = whitelisted_token.decimals;

        let minimum_unlock_price = Price {
            multiplier: token_args.target_price.0,
            decimals: token_args.metadata.decimals
        };

        assert_eq!(token_decimals + 4, token_args.metadata.decimals, "Wrong decimals");
        assert!(token_args.target_price.0 > 10000, "Wrong target price {}", token_args.target_price.0);

        //let target_price_short: u128 = (token_args.target_price.0 * 10_u128.pow((token_args.metadata.decimals - token_decimals) as u32));
        let target_price_short: u128 = token_args.target_price.0 / 10000;
        assert!(target_price_short > 0, "Wrong target price {}", target_price_short);

        token_args.metadata.name = format!("{} to ${}", token_name, target_price_short);
        token_args.metadata.symbol = format!("{}@{}", token_name, target_price_short);

        let token_id = format!("{}_{}", token_name, token_args.target_price.0).to_ascii_lowercase();
        assert!(is_valid_token_id(&token_id), "Invalid Symbol");
        let token_account_id = format!("{}.{}", token_id, env::current_account_id());
        assert!(
            env::is_valid_account_id(token_account_id.as_bytes()),
            "Token Account ID is invalid"
        );


        let args: TokenArgs = TokenArgs {
            locked_token_account_id: token_account_id.clone(),
            meta: token_args.metadata,
            backup_trigger_account_id: token_args.backup_trigger_account_id.map(|a| a.into()),
            price_oracle_account_id: token_args.price_oracle_account_id.into(),
            asset_id: token_args.token_id.into(),
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
            "Token ID {} is already taken", token_id
        );

        let storage_balance_used =
            Balance::from(env::storage_usage() - initial_storage_usage) * STORAGE_PRICE_PER_BYTE;

        Promise::new(token_account_id)
            .create_account()
            .transfer(required_balance - storage_balance_used)
            .deploy_contract(FT_WASM_CODE.to_vec())
            .function_call(b"new".to_vec(),
                           serde_json::to_vec(&args).unwrap(), 0, GAS)
    }
}

pub mod u64_dec_format {
    use near_sdk::serde::de;
    use near_sdk::serde::{Deserialize, Deserializer, Serializer};

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
    use near_sdk::serde::de;
    use near_sdk::serde::{Deserialize, Deserializer, Serializer};

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

