//! Navigation
//!
//! Defines navigation systems, inertial mechanics, and stellar weather interactions.

pub mod chronological_stutter;
pub mod inertial;
pub mod stellar_weather;
pub mod wind_surfing;

pub use chronological_stutter::*;
pub use inertial::*;
pub use stellar_weather::*;
pub use wind_surfing::{
    flare_exposure, send_wind_flare, wind_surf_stats_counts, FleetRadiationExposure,
    FleetShielding, StellarWindFlare, WindSurfLedger, WindSurfer, DEFAULT_FLARE_INTENSITY,
};

pub mod warp_wake;
pub use warp_wake::*;
