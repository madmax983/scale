//! Trade
//!
//! Handles inter-system trade, trade routes, tariffs, and blockades.

pub mod biomass_tariff;
pub mod feral_logistics;

pub use biomass_tariff::*;
pub mod blockade;
pub mod escape_velocity;
pub mod merchant_landings;
pub mod penal_contracts;
pub mod phantom_limb_logistics;
pub mod phantom_trade_routes;
pub mod routes;
pub mod smuggling;

use bevy::prelude::*;

pub struct TradePlugin;

impl Plugin for TradePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(phantom_trade_routes::PhantomTradeRoutePlugin);
    }
}
