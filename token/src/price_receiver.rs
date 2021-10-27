use crate::*;
use near_sdk::{Duration, Timestamp};
use std::cmp::Ordering;

pub type AssetId = String;
pub type DurationSec = u32;

const MAX_U128_DECIMALS: u8 = 38;
const UNLOCKING_DURATION: Duration = 24 * 60 * 60 * 10u64.pow(9);

#[derive(Serialize, Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct AssetOptionalPrice {
    pub asset_id: AssetId,
    pub price: Option<Price>,
}

#[derive(Serialize, Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct PriceData {
    #[serde(with = "u64_dec_format")]
    pub timestamp: Timestamp,
    pub recency_duration_sec: DurationSec,

    pub prices: Vec<AssetOptionalPrice>,
}

pub trait OraclePriceReceiver {
    fn oracle_on_call(&mut self, sender_id: AccountId, data: PriceData, msg: String);
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Clone, Copy)]
#[serde(crate = "near_sdk::serde")]
pub struct Price {
    #[serde(with = "u128_dec_format")]
    multiplier: Balance,
    decimals: u8,
}

impl PartialEq<Self> for Price {
    fn eq(&self, other: &Self) -> bool {
        self.partial_cmp(other) == Some(Ordering::Equal)
    }
}

impl PartialOrd for Price {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        if self.decimals < other.decimals {
            return other.partial_cmp(self).map(|o| o.reverse());
        }

        let decimals_diff = self.decimals - other.decimals;

        if decimals_diff > MAX_U128_DECIMALS {
            return Some(Ordering::Less);
        }

        if let Some(om) = other
            .multiplier
            .checked_mul(10u128.pow(decimals_diff as u32))
        {
            Some(self.multiplier.cmp(&om))
        } else {
            Some(Ordering::Less)
        }
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

#[near_bindgen]
impl OraclePriceReceiver for Contract {
    #[allow(unused_variables)]
    fn oracle_on_call(&mut self, sender_id: AccountId, data: PriceData, msg: String) {
        assert_eq!(
            &env::predecessor_account_id(),
            &self.price_oracle_account_id
        );
        for AssetOptionalPrice { asset_id, price } in data.prices {
            if asset_id == self.asset_id {
                if let Some(price) = price {
                    if price >= self.minimum_unlock_price {
                        log!("maybe_unlock {}/{} >= {}/{}", price.multiplier, price.decimals, self.minimum_unlock_price.multiplier, self.minimum_unlock_price.decimals);
                        self.maybe_unlock();
                        return;
                    }
                    log!("maybe_lock {}/{} < {}/{}", price.multiplier, price.decimals, self.minimum_unlock_price.multiplier, self.minimum_unlock_price.decimals);
                }
                self.maybe_lock();
                return;
            }
        }
    }
}

impl Contract {
    pub fn maybe_unlock(&mut self) {
        match self.status {
            Status::Locked => {
                let initiated_timestamp = env::block_timestamp();
                self.status = Status::Unlocking {
                    initiated_timestamp,
                };
                log!(
                    "Started unlocking at {}, unlocks at {}",
                    initiated_timestamp,
                    initiated_timestamp + UNLOCKING_DURATION
                );
            }
            Status::Unlocking {
                initiated_timestamp,
            } => {
                let timestamp = env::block_timestamp();
                if initiated_timestamp + UNLOCKING_DURATION > timestamp {
                    log!(
                        "Still unlocking, unlocks at {}, but current time is {}",
                        initiated_timestamp + UNLOCKING_DURATION,
                        timestamp
                    );
                } else {
                    log!("Unlocked!");
                    self.status = Status::Unlocked;
                }
            }
            Status::Unlocked => {
                env::panic(b"Already unlocked");
            }
        }
    }

    pub fn maybe_lock(&mut self) {
        match self.status {
            Status::Locked => {
                env::panic(b"Still locked");
            }
            Status::Unlocking { .. } => {
                self.status = Status::Locked;
                log!("Locked again");
            }
            Status::Unlocked => {
                env::panic(b"Already unlocked");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(multiplier: u128, decimals: u8) -> Price {
        Price {
            multiplier,
            decimals,
        }
    }

    #[test]
    pub fn test_price_cmp() {
        assert!(p(10, 0) < p(11, 0));
        assert!(p(11, 0) > p(10, 0));
        assert!(p(11, 0) == p(11, 0));

        assert!(p(10, 10) < p(11, 10));
        assert!(p(11, 10) > p(10, 10));
        assert!(p(11, 10) == p(11, 10));

        assert!(p(100, 10) == p(10, 9));
        assert!(p(10, 9) == p(100, 10));

        assert!(p(101, 10) > p(10, 9));
        assert!(p(10, 9) < p(101, 10));
        assert!(p(99, 10) < p(10, 9));
        assert!(p(10, 9) > p(99, 10));

        assert!(p(101, 40) < p(10, 0));
        assert!(p(10, 0) > p(101, 40));
    }
}
