use crate::*;

#[near_bindgen]
impl TokenFactory {
    #[private]
    #[init(ignore_state)]
    #[allow(dead_code)]
    pub fn migrate_1() -> Self {
        #[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
        #[serde(crate = "near_sdk::serde")]
        pub struct WhitelistedTokenOld {
            pub title: String,
            pub asset_id: String,
            pub decimals: u8,
        }

        #[derive(BorshDeserialize)]
        pub struct TokenFactoryOld {
            pub tokens: UnorderedMap<TokenId, TokenArgs>,
            pub storage_deposits: LookupMap<AccountId, Balance>,
            pub storage_balance_cost: Balance,
            pub whitelisted_tokens: UnorderedMap<AccountId, WhitelistedTokenOld>,
        }

        let old_contract: TokenFactoryOld = env::state_read().expect("Old state doesn't exist");

        let mut whitelisted_tokens_new: UnorderedMap<AccountId, WhitelistedToken> = UnorderedMap::new(StorageKey::WhitelistedTokensV1);

        let token_ids = old_contract.whitelisted_tokens.keys_as_vector();
        for token_id in token_ids.to_vec() {
            if let Some(old_token) = old_contract.whitelisted_tokens.get(&token_id) {
                whitelisted_tokens_new.insert(&token_id,
                                              &WhitelistedToken {
                                                  asset_id: old_token.asset_id,
                                                  metadata: FungibleTokenMetadata {
                                                      spec: "ft-1.0.0".to_string(),
                                                      name: old_token.title,
                                                      symbol: "".to_string(),
                                                      icon: None,
                                                      reference: None,
                                                      reference_hash: None,
                                                      decimals: old_token.decimals,
                                                  },
                                              });
            }
        }

        TokenFactory {
            tokens: old_contract.tokens,
            storage_deposits: old_contract.storage_deposits,
            storage_balance_cost: old_contract.storage_balance_cost,
            whitelisted_tokens: whitelisted_tokens_new,
        }
    }
}
