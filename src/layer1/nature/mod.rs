pub mod long_night;
pub use long_night::*;
/// Nature
///
/// Handles ecology, terrain generation, weather, and natural phenomena.
pub mod atmosphere;
pub mod atmospheric_empathy;
pub use atmospheric_empathy::*;
#[cfg(test)]
mod atmosphere_corrosion_tests;
#[cfg(test)]
mod atmosphere_tides_tests;
pub mod biosphere_empathy;
pub mod ecology;
#[cfg(test)]
mod ecology_keystone_tests;
#[cfg(test)]
mod ecology_tests;
pub mod erosion;
pub mod fertility;
pub mod fire;
pub mod megafauna_terrain;
pub mod psychoactive_weather;
pub mod radioactive;
pub mod seasons;
pub mod solar;
pub mod temperature;
pub mod terrain;
pub mod water;
pub use psychoactive_weather::*;

pub mod weather;
#[cfg(test)]
mod weather_tests;
pub mod wind;

// Spec 337 Symbiotic Pests (deliberately NOT glob-exported: the
// `Vermin` name is already taken by Spec 073's marker at
// `crate::layer1::entities::vermin::Vermin`, glob-exported at the
// `crate::layer1` root. Full path only.)
pub mod symbiotic_pests;

pub use atmosphere::*;
pub use biosphere_empathy::*;
pub use ecology::*;
pub use erosion::*;
pub use fertility::*;
pub use fire::*;
pub use megafauna_terrain::*;
pub use radioactive::*;
pub use seasons::*;
pub use solar::*;
pub use temperature::*;
pub use terrain::*;
pub use water::*;
pub use weather::*;
pub mod solar_flare_lottery;
pub use solar_flare_lottery::*;
pub use wind::*;
pub mod mutagenic_rain;
pub use mutagenic_rain::*;

#[cfg(test)]
mod silent_world_tests;

pub mod crustal_tides;
pub use crustal_tides::*;
pub mod shattered_world;
pub use shattered_world::*;

pub mod archaeology;
pub use archaeology::*;

pub mod subterranean_smog;
pub use subterranean_smog::*;

pub mod biosphere_inversion;
pub use biosphere_inversion::*;

pub mod biome_aggression;
pub use biome_aggression::*;
