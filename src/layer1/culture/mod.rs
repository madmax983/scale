//! Culture
//!
//! Defines cultural aspects like art, festivals, funerals, and artifacts.

/// Ancestral Graves system.
pub mod ancestral_graves;
pub use ancestral_graves::*;

/// Cultural Artifacts and Statues.
pub mod art;
pub use art::*;

/// Xeno-Artifacts system (Spec 156).
pub mod artifacts;
pub use artifacts::*;

/// Emergent Festivals system (Spec 077).
pub mod festivals;
pub use festivals::*;

/// Funeral rites and corpse management (Spec 057).
pub mod funeral;
pub use funeral::*;

/// Totems and superstition system (Spec 200).
pub mod totems;
pub use totems::*;
pub mod animism;
pub use animism::*;
pub mod astrology;
pub use astrology::*;
pub mod gastronomers;
pub use gastronomers::*;

pub mod cultural_artifacts;
pub use cultural_artifacts::*;

pub mod cult_of_first_ship;
pub use cult_of_first_ship::*;

pub mod nostalgia;
pub use nostalgia::*;
pub mod linguistics;
pub use linguistics::*;
pub mod celestial_cemeteries;
pub use celestial_cemeteries::*;

pub mod cultural_influence;
pub use cultural_influence::*;
pub mod invasive_xeno_aesthetics;
pub mod memorial_revolt;

/// The Fallen Sovereign: exiled-monarch adventurer origin.
pub mod sovereign;
pub use sovereign::*;
/// The Corsair: space-pirate adventurer origin.
pub mod corsair;
pub use corsair::*;
/// The Planetary Governor: bureaucratic adventurer origin.
pub mod governor;
pub use governor::*;
/// The Salvager: wreck-diver adventurer origin.
pub mod salvager;
pub use salvager::*;
/// The Improbable Pilot: Longshot Drive adventurer origin.
pub mod improbable;
pub use improbable::*;
/// The Lawbound: Three-Statutes automaton adventurer origin.
pub mod lawbound;
pub use lawbound::*;
pub use invasive_xeno_aesthetics::*;
