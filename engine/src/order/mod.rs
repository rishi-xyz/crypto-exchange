use std::{collections::VecDeque, sync::Arc};

use crate::types::{OrderId, OrderStatus, OrderType, Price, Quantity, Side};

pub struct Order {
    order_id: OrderId,
    order_type: OrderType,
    side: Side,
    status: OrderStatus,
    price: Price,
    intial_quantity: Quantity,
    remaining_quantity: Quantity,
    timestamp: u64
}

impl Order {
    pub fn new(
        order_id: OrderId,
        order_type: OrderType,
        side: Side,
        status: OrderStatus,
        price: Price,
        quantity: Quantity,
    ) ->Self {
        return Self { 
            order_id, 
            order_type, 
            side, 
            status, 
            price, 
            intial_quantity: (quantity) , 
            remaining_quantity: (quantity), 
            timestamp: (0) 
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
        return  self.intial_quantity
    }

    pub fn get_remaining_quantity(&self) ->Quantity {
        return  self.remaining_quantity
    }

    pub fn get_filled_quantity(&self) ->Quantity {
        return self.intial_quantity - self.remaining_quantity;
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
        Ok(())
    }
}

pub type OrderPointer = Arc<Order>;
pub type OrderPointers = VecDeque<OrderPointer>;