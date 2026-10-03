//! Layer 1: Colony Simulation.
//!
//! This is the "Dwarf Fortress" or "RimWorld" layer of the game. It simulates the daily life
//! of the colony on a tile-based grid.
//!
//! # Core Concepts
//!
//! ## The Grid
//! The world is represented by a 2D grid of tiles (see [`crate::layer1::map::GridPosition`] and [`crate::layer1::terrain::TerrainGrid`]).
//! Each tile can contain:
//! - **Terrain:** The base layer (Grass, Water, Rock).
//! - **Building:** Constructed structures (Housing, Farm, Walls). See [`crate::layer1::architecture::building`].
//! - **Entities:** Pops, Visitors, Fauna, and Items.
//!
//! ## The Agents (Pops)
//! "Pops" are the primary agents. They are not directly controlled by the player. Instead, they:
//! 1.  **Have Needs:** Hunger, Rest, Social, Leisure (see [`crate::layer1::psychology::needs`]).
//! 2.  **Make Decisions:** Utility AI scores potential actions based on needs and environment (see [`crate::layer1::mind::utility_ai`]).
//! 3.  **Perform Actions:** Working, Eating, Sleeping, Socializing (see [`crate::layer1::actions`]).
//!
//! ## The Simulation Loop
//! The simulation advances in discrete ticks (see [`crate::simulation`]).
//! 1.  **Decision Phase:** AI systems evaluate options and assign `PopAction`s.
//! 2.  **Execution Phase:** Systems like `movement_system` and `work_execution_system` progress these actions.
//! 3.  **Economy Phase:** Resources are produced/consumed, needs decay.
//!
//! # Module Structure
//! - **Entities:** [`crate::layer1::entities::pop`], [`crate::layer1::architecture::building`], [`crate::layer1::fauna`], [`crate::layer1::entities::visitor`]
//! - **Systems:** [`crate::layer1::psychology::needs`], [`crate::layer1::biology::health`], [`crate::layer1::execution::combat`], [`crate::layer1::tech`]
//! - **Environment:** [`crate::layer1::nature::terrain`], [`crate::layer1::core::map`], [`crate::layer1::nature::weather`], [`crate::layer1::lighting`]
//! - **Economy:** [`crate::layer1::economy::resources`], [`crate::layer1::economy::trade`]

pub mod disasters;
pub mod mycelial;
pub use mycelial::*;

pub mod cryo_prison;
pub mod petrification;

/// Generic entities mechanisms
pub mod entities;
pub use entities::*;
/// Architecture simulation mechanics
pub mod architecture;
pub use architecture::*;
/// Economy mechanisms
pub mod economy;
pub use economy::*;
pub mod access_control;
/// Pop actions logic.
pub mod actions;
pub mod mind;
pub mod tinkering;
pub use mind::*;
pub mod ad_screen;
/// Bureaucratic Drag system (Spec 175).
pub mod memorial_economy;
pub use memorial_economy::*;
pub mod administration;
pub use administration::*;
/// Game balance constants.
pub mod balance;
/// Beauty and decoration system.
pub mod beauty;
/// Pop biography system.
pub mod biography;
/// Cultural, Religious, and Belief systems.
pub mod culture;
pub use culture::*;
/// Building placement and types.
/// Chronicle system and historical records.
pub mod clothing;
/// Door control system (Spec 134).
/// Crowding system (Spec 176).
pub mod crowding;
/// Defensive structures and logic.
pub mod defense;
/// Designation system for player tools.
/// Ecological succession system (Spec 161).
/// Colony policies and edicts.
/// Execution layer bridging utility AI to actions.
pub mod execution;
/// Historical geography naming.
pub mod geography;
/// Farm building and food production.
/// Pop health and damage.
/// Deep crust geomes system (Spec 515).
/// Fire propagation and damage.
/// Hauling logic.
/// Tests for hazards logic.
/// Pop hobbies logic (Spec 137).
pub mod hobby;

/// Housing and rest mechanics.
/// The Inspector system (Spec 091).
/// Institutional Memory system (Spec 172).
pub mod institutional_memory;
/// Personal inventory system.
/// Item definitions (Tools, Equipment).
pub mod law;
pub use law::orphaned_edict::*;
pub mod improvised_tools;
/// Colony Mascot system (Spec 129).
/// Medical care and hospital logic.
pub mod jobs;
pub use improvised_tools::*;
/// Tests for metal industry (Spec 024).
pub mod metal_industry_tests;
/// Terrain generation and grid management.
/// Tests for mining logic (Spec 052).
pub mod mining_tests;
/// Pop needs (hunger, rest).
pub mod psychology;
pub use psychology::*;
/// Pop entity and management.
/// Refining industry (Lumber Mill, Stone Mason).
/// Colony resources and mining.
/// Field science and anomalies.
pub mod anomalies;
/// Notification system.
pub mod notifications;
/// Pathfinding algorithms.
pub mod pathfinding;
/// Seasonal rhythms (Spring, Summer, Autumn, Winter).
mod shift_integration_tests;
/// Social needs and tavern.
pub mod sonic_suppression;
pub use sonic_suppression::*;

pub mod social;
/// Spoilage and decay mechanics.
pub mod spoilage;
/// Resource storage limits and stockpile buildings.
/// Structure durability and repair.
/// Technology and research system.
pub mod tech;

/// Tests for mother lode logic (Spec 168).
pub mod mother_lode_tests;

mod execution_demolish_test;

pub mod temporal_chamber;
pub use temporal_chamber::*;

/// Named locations on the map.
pub mod locations;

/// Graffiti and Signage system (Spec 144).
pub mod graffiti;

pub use access_control::*;
pub use actions::*;
pub use balance::*;
pub use beauty::*;
pub use biography::*;
pub use crowding::*;
pub use defense::*;
pub mod biology;
pub use biology::*;
pub use execution::*;

pub use anomalies::*;
pub use graffiti::*;
pub use hobby::*;
pub use institutional_memory::*;
pub use jobs::*;
pub use locations::*;
pub use notifications::*;
pub use social::*;
pub use spoilage::*;
pub use tech::*;

mod tool_tests;
pub use clothing::*;

/// Lighting system.
pub mod lighting;
/// Pop skills and experience system.
pub mod skills;

mod waste_tests;

/// Vermin infestation logic (Spec 073).
pub use lighting::*;

/// Trade system.
/// Energy system (Spec 042).
pub mod energy;
pub use energy::*;

/// Pop lifecycle and aging (Spec 062).
pub mod lifecycle;
/// Whispering ore deposits mechanics
pub mod whispering_ore;
pub use lifecycle::*;
pub use whispering_ore::*;
/// Atmospheric simulation (Spec 063).
/// Room quality calculation and memories (Spec 064).
/// Radio Nostalgia system (Spec 814).
pub mod radio_nostalgia;
pub use radio_nostalgia::*;

/// Ruins system (Spec 167).
/// Hostile Fauna (Spec 048).
pub mod fauna;
pub use fauna::*;
pub mod chain_of_command;
/// Procedural Fauna Generation (Spec 164).
/// Combat system and drafting logic.
pub mod combat;
/// Visitor system (Spec 074).
/// The Visitor (Mega-Fauna) system (Spec 234).
/// Day/Night cycle system (Spec 065).
pub mod day_night;
pub use day_night::*;

mod material_provenance_tests;

/// Stowaway system (Spec 086).
pub mod stowaway;
pub use stowaway::*;
/// Erosion system (Spec 093).
/// Eureka Moments system (Spec 196).
pub mod eureka;
pub use eureka::*;

/// Pheromone Gardening system (Spec 170).
pub mod pheromone;
pub use pheromone::*;

/// Desire Dust roads / The Sentient Commute (Spec 1379).
pub mod desire_dust;
pub use desire_dust::*;

/// Heirloom tech system (Spec 070).
pub mod heirloom;
pub mod heirloom_tool;
pub use heirloom::*;
pub use heirloom_tool::*;

mod heirloom_items_tests;

mod heirloom_tests;

mod retrograde_tests;

/// Animal Husbandry system (Spec 075).
/// Gastronomy system (Spec 166).
/// Antagonistic Flora system (Spec 092).
pub mod flora;
pub use flora::*;
/// Gut Biome system (Spec 211).
/// Private stash system for pops.
pub mod private_stash;
pub use private_stash::*;
/// Resource purity system (Spec 106).
pub mod purity;
pub use purity::*;

mod fuel_consumption_tests;

mod fuel_industry_tests;

/// Palette fatigue system (Spec 114).
pub mod palette_fatigue;

pub use palette_fatigue::*;

/// Spontaneous Architecture system (Spec 110).
/// Observatory and Overview Effect (Spec 115).
pub mod observatory;
pub use observatory::*;

/// Conveyor and Hopper Logistics (Spec 111).
pub mod logistics;
pub use logistics::*;

/// Turret system (Spec 135).
/// Wild Child system (Spec 124).
pub mod prototyping;

mod tech_storage_tests;
pub use prototyping::*;
pub mod geology;
pub use geology::*;

#[cfg(feature = "nova")]
pub mod oral_tradition;
#[cfg(feature = "nova")]
pub use oral_tradition::*;

#[cfg(not(feature = "nova"))]
pub mod oral_tradition_stub;
#[cfg(not(feature = "nova"))]
pub use oral_tradition_stub::*;

/// Genius Loci system (Nova Feature).
#[cfg(feature = "nova")]
pub mod loci;
#[cfg(feature = "nova")]
pub use loci::*;

/// Constellation Mythology system (Nova Feature).
#[cfg(feature = "nova")]
pub mod constellations;
#[cfg(feature = "nova")]
pub use constellations::*;

pub mod tech_envy;

/// Drone Networks (Spec 116).
/// The Observer Effect (Nova Feature).
#[cfg(feature = "nova")]
pub mod observer;
#[cfg(feature = "nova")]
pub use observer::*;

pub mod systems;

pub mod memetics;
/// Orbital Crossfire system (Spec 206).
/// Scrapcode virus system (Spec 178).
pub mod scrapcode;
pub use scrapcode::*;

/// Chemical regulation system (Spec 181).
pub mod chemical;
/// Cryo-Stasis system (Spec 139).
pub mod cryo;
pub use chemical::*;
pub use cryo::*;

#[cfg(test)]
mod cryo_tests;

/// Radioactive system (Spec 191).
/// Company Scrip and Economy system (Spec 194).
/// Language and Dialect system (Spec 193).
pub mod language;
pub use language::*;

mod equipment_tests;

/// Solar cycle and power generation (Spec 213).
/// Hygiene system (Spec 220).
pub mod hygiene;
pub use hygiene::*;

/// Gene Bank system (Spec 165).
pub mod geodetic;

mod improvised_tools_tests;

mod urban_heat_tests;
pub use geodetic::*;

/// The Mother Lode system (Spec 168).
pub mod mother_lode;
pub use mother_lode::*;

/// Organic Recycling system (Spec 221).
pub mod recycling;
pub use recycling::*;
pub mod corporate_sponsorship;
/// Direct Link (Possession) system (Spec 236).
pub mod direct_link;
pub use corporate_sponsorship::*;
/// Permit system for advanced construction.
pub use direct_link::*;

/// Void Signals (Nova Feature).
#[cfg(feature = "nova")]
pub mod void_signals;
#[cfg(feature = "nova")]
pub use void_signals::*;
/// The Hum system (Spec 238).
pub mod hum;
pub use hum::*;

/// Operational Detritus system (Spec 239).
pub mod clutter;
pub use clutter::*;

/// Biometric Security system (Spec 244).
pub mod security;
pub use security::*;

mod geodetic_tests;

/// Quantum Twins (Spec 245).
pub mod quantum_twins;
pub use quantum_twins::*;

/// Holographic Facades (Spec 249).
/// Construction mechanics
pub mod construction;
/// Holographic projections
pub mod hologram;

mod hologram_tests;
pub use hologram::*;

/// Memory core structures
pub mod memory_core;
pub use memory_core::*;

/// Olfactory map and scent system (Spec 446).
pub mod olfactory;
pub use olfactory::*;

/// Simulation physics engines
pub mod physics;
pub use physics::*;

/// Nature mechanics
pub mod nature;
pub use nature::*;
/// Shipbreaking tools
pub mod shipbreaking;
pub use shipbreaking::*;
/// Void weed flora system
pub mod void_weed;
pub use void_weed::*;

/// Temporal Ghost Towns mechanics
pub mod temporal_ghost_towns;
pub use temporal_ghost_towns::*;

/// Infrastructure system (Spec 452).
pub mod infrastructure;
pub use infrastructure::*;

/// Nanite Fabrication system (Spec 453).
pub mod nanite_fabrication;
pub use nanite_fabrication::*;
/// Exodus mechanisms for pops leaving
pub mod exodus;
pub use exodus::*;
/// Generic environment mechanics
pub mod environment;
/// Spore diplomat faction mechanics
pub mod spore_diplomat;
pub use spore_diplomat::*;
/// Agriculture and Food Production
pub mod agriculture;
/// Diplomacy systems
pub mod diplomacy;
pub use agriculture::*;
/// Haunted assembly lines events
pub mod haunted_assembly_lines;
/// Local tributes logic
pub mod local_tributes;
/// Religion systems
pub mod religion;
/// Unseen bureaucracy event logic
pub mod unseen_bureaucracy;
pub use unseen_bureaucracy::*;
/// Core layer mechanisms
pub mod core;
pub use core::*;
pub mod combat_stats;
pub mod deep_crust_resonance;
/// Digital immortality mechanics
pub mod digital_immortality;
pub mod rearguard;
pub mod shields;
//pub use combat_stats::*;
pub use rearguard::{
    rearguard_buff_system, rearguard_delays_enemy_system, rearguard_pathfinding_restriction_system,
    Enemy, EngagedWith, EscapePod, Pathfinding, Rearguard,
};
pub mod sub_glacial_oceans;

pub mod nanite_storms;
pub mod predecessors;
pub use nanite_storms::*;

/// Foundation soil feature (Spec 1078)
pub mod foundation_soil;
pub use foundation_soil::*;

pub mod cassandra_protocol;
pub mod cassandra_syndrome;
pub use cassandra_protocol::*;
pub use cassandra_syndrome::*;
pub mod ransom_broker;
pub use ransom_broker::*;

pub mod void_sirens;
pub use void_sirens::*;
pub mod living_score;
pub use living_score::*;
pub mod archaeological_contagion;
pub mod bureaucracy_of_scarcity;
pub use bureaucracy_of_scarcity::{
    apply_rationing_buff_system, evaluate_scarcity_system, ConsumptionRate, GlobalRationingModifier,
};
pub mod architecture_sentience;
pub use architecture_sentience::*;
pub mod shipbreaking_symbiotic;
pub mod specialization;
pub use shipbreaking_symbiotic::*;
pub mod orphaned_swarm;
pub use orphaned_swarm::*;

pub mod architecture_superstition;
pub use architecture_superstition::*;

pub mod artists_muse;
pub mod crafting;
pub mod fungal_network;
pub mod hive_mind_integration;
pub use fungal_network::{process_spore_taps_system, PopCollectivism, SporeNetwork, SporeTap};
pub mod the_lottery;

// Echo DX Audit: Provide intuitive module aliases for users expecting flattened imports
pub mod buildings {
    pub use crate::layer1::architecture::building::*;
}
#[cfg(test)]
pub mod clothing_tests;
pub mod mobile_architecture;

pub mod blackout_bazaars;
pub use blackout_bazaars::{
    bazaar_trading_system, despawn_blackout_bazaars_system, spawn_blackout_bazaars_system,
    BazaarInventory, BlackoutBazaar, PlayerTradeEvent, RareItem, SocialArea,
};
pub mod terraforming;
pub mod terraforming_rejection;
pub use terraforming::*;
pub use terraforming_rejection::*;
pub mod feral_overlord;
pub mod shadow_ecosystems;
pub use shadow_ecosystems::*;
pub mod hazards;
pub use hazards::*;

pub mod heroic_acts;
pub use heroic_acts::*;
pub mod gravity_funerals;
pub use gravity_funerals::*;
pub mod spatial_compression;
pub use spatial_compression::*;
pub mod leader_ascension;
pub use leader_ascension::*;
pub mod accidental_terraforming;
pub use accidental_terraforming::*;
pub mod consultant;
pub use consultant::*;

pub struct ConsultantPlugin;
impl bevy::prelude::Plugin for ConsultantPlugin {
    fn build(&self, app: &mut bevy::prelude::App) {
        app.add_systems(
            bevy::prelude::Update,
            (
                consultant::apply_consultant_override,
                consultant::consultant_worker_impact,
                consultant::consultant_hazard_escalation,
            ),
        );
    }
}
