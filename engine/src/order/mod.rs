use std::{collections::VecDeque, sync::{Arc, Mutex}, time::{SystemTime, UNIX_EPOCH}};
use serde::{Deserialize, Serialize};

use crate::types::{OrderId, OrderStatus, OrderType, Price, Quantity, Side, UserId};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Order {
    order_id: OrderId,
    order_type: OrderType,
    side: Side,
    status: OrderStatus,
    price: Price,
    initial_quantity: Quantity,
    remaining_quantity: Quantity,
    timestamp: u64,
    user_id: UserId
}

impl Order {
    pub fn new(
        order_id: OrderId,
        order_type: OrderType,
        side: Side,
        status: OrderStatus,
        price: Price,
        quantity: Quantity,
        user_id: UserId
    ) ->Self {
        return Self { 
            order_id, 
            order_type, 
            side, 
            status, 
            price, 
            initial_quantity: (quantity) , 
            remaining_quantity: (quantity), 
            timestamp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos() as u64,
            user_id
        };
    }
    
    pub fn get_order_id(&self) ->OrderId {
        return  self.order_id
    }

    pub fn get_type(&self) ->OrderType {
        return  self.order_type
    }

    pub fn get_side(&self) ->Side {
        return  self.side
    }

    pub fn get_status(&self) ->OrderStatus {
        return  self.status
    }

    pub fn get_price(&self) ->Price {
        return  self.price
    }

    pub fn get_initial_quantity(&self) ->Quantity {
        return  self.initial_quantity
    }

    pub fn get_remaining_quantity(&self) ->Quantity {
        return  self.remaining_quantity
    }

    pub fn get_filled_quantity(&self) ->Quantity {
        return self.initial_quantity - self.remaining_quantity;
    }

    pub fn is_filled(&self) ->bool {
        return self.get_remaining_quantity() == 0;
    }

    pub fn fills(&mut self,quantity: Quantity) ->Result<(),String> {
        if quantity > self.get_remaining_quantity() {
            return  Err(format!(
                "Order ({}) cannot be filled for more than its remaining quantity",
                self.get_order_id()
            ));
        }
        self.remaining_quantity -= quantity;
        if self.remaining_quantity == 0 {
            self.status = OrderStatus::Filled;    
        }else {
            self.status = OrderStatus::PartiallyFilled;   
        }
        Ok(())
    }

    pub fn get_timestamp(&self) -> u64 {
        self.timestamp
    }

    pub fn get_user_id(&self) -> UserId {
        self.user_id
    }
}

pub type OrderPointer = Arc<Mutex<Order>>;
pub type OrderPointers = VecDeque<OrderPointer>;