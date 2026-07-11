use std::sync::Arc;

use crate::{order::Order, types::{OrderId, OrderStatus, OrderType, Price, Quantity, Side, UserId}};

pub struct OrderModify {
    order_id: OrderId,
    price: Price,
    side: Side,
    quantity: Quantity,
    status: OrderStatus,
    user_id: UserId
}

impl OrderModify {
    pub fn new(
        order_id: OrderId,
        price: Price,
        side: Side,
        quantity: Quantity,
        status: OrderStatus,
        user_id: UserId
    ) ->Self {
        return Self { order_id, price, side, quantity, status, user_id };
    }   

    pub fn get_order_id(&self) ->OrderId {
        return self.order_id
    }

    pub fn get_price(&self) ->Price {
        return self.price
    }

    pub fn get_side(&self) ->Side {
        return self.side
    }
    pub fn get_quantity(&self) ->Quantity {
        return  self.quantity
    }

    pub fn get_status(&self) ->OrderStatus {
        return  self.status
    }

    pub fn get_user_id(&self) ->UserId {
        return  self.user_id
    }

    pub fn to_order_pointer(&self,order_type: OrderType) ->Arc<Order>{
        return Arc::new(Order::new(self.order_id, order_type, self.side, self.status, self.price, self.quantity, self.user_id));
    }
}