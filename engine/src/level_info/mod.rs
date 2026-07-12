use std::{collections::VecDeque, sync::Arc};

use serde::{Deserialize, Serialize};

use crate::types::{Price, Quantity};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LevelInfo {
    pub price : Price,
    pub quantity : Quantity
}

impl LevelInfo {
    pub fn new(price:Price, quantity:Quantity) ->Self {
        LevelInfo { price, quantity }
    }
}

pub type LevelInfos = Arc<VecDeque<LevelInfo>>;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderBookLevelInfo {
    bids : LevelInfos,
    asks : LevelInfos,
}

impl OrderBookLevelInfo {
    pub fn new(bids_ :LevelInfos,asks_ :LevelInfos) ->Self {
        return OrderBookLevelInfo { 
            bids: bids_, 
            asks: asks_ 
        };
    }
    pub fn get_bids(&self) ->LevelInfos {
        return self.bids.clone();
    }

    pub fn get_asks(&self) ->LevelInfos {
        return  self.asks.clone();
    }
}