//! # Pop Needs
//!
//! This module simulates the physiological and psychological needs of every Pop in the colony.
//! Needs are the primary drivers of behavior in the [Utility AI](crate::layer1::utility_ai) system.
//!
//! ## Core Needs
//!
//! Each Pop has three core needs, represented as floating-point values from **0.0** (Critical) to **1.0** (Satisfied).
//!
//! 1.  **Hunger**:
//!     *   Decays over time (approx. 1000 ticks from full to starvation).
//!     *   Replenished by eating food (Action: `SatisfyHunger`).
//!     *   **Consequence**: At 0.0, Pops take starvation damage and eventually die.
//!
//! 2.  **Rest**:
//!     *   Decays over time (approx. 1000 ticks from rested to exhausted).
//!     *   Replenished by sleeping (Action: `SatisfyRest`).
//!     *   **Consequence**: Low rest reduces movement speed and work efficiency.
//!
//! 3.  **Leisure**:
//!     *   Decays slightly faster than other needs.
//!     *   Replenished by socializing, praying, or entertainment (Action: `Socialize`).
//!     *   **Consequence**: Low leisure contributes to stress and mental breaks.
//!
//! ## Decay Mechanics
//!
//! Every tick, `decay_needs_system` reduces these values based on constants and modifiers:
//!
//! *   **Base Decay**: Fixed rate per tick (e.g., `0.001` for Hunger).
//! *   **Traits**: [Traits](crate::layer1::traits) like `Glutton` increase hunger decay.
//! *   **Policies**: [Policies](crate::layer1::edicts) like `Rationing` reduce hunger decay at the cost of morale.
//!
//! ## Morale
//!
//! "Morale" is the aggregate score of all needs. High morale grants efficiency bonuses, while low morale
//! leads to mental breaks (tantrums, depression).

use crate::layer1::edicts::{get_hunger_decay_modifier, ColonyPolicies};
use crate::layer1::health::Health;
use crate::layer1::memory::{Memories, MemoryType};
use crate::layer1::traits::{
    get_trait_hunger_decay_modifier, get_trait_leisure_decay_modifier, Traits,
};
use bevy_ecs::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NeedType {
    Hunger,
    Rest,
    Leisure,
    Hygiene,
    Food, // Alias used in spec tests
}

/// Pop survival needs.
///
/// Tracks the physical and mental state of a citizen. Values range from 0.0 (Empty/Critical) to 1.0 (Full/Satisfied).
///
/// # Default Values
///
/// New pops start with needs at **0.8** (80%), giving them a buffer before needing to act.
#[derive(Component, Clone, Copy, Debug)]
pub struct Needs {
    /// Hunger level.
    /// *   **1.0**: Full belly.
    /// *   **< 0.2**: Hungry (Urgent).
    /// *   **0.0**: Starving (Taking damage).
    pub hunger: f32,

    /// Rest level.
    /// *   **1.0**: Fully rested.
    /// *   **< 0.2**: Exhausted (Movement penalty).
    /// *   **0.0**: Collapsing.
    pub rest: f32,

    /// Leisure/Social level.
    /// *   **1.0**: Entertained.
    /// *   **< 0.2**: Bored/Stressed.
    pub leisure: f32,

    /// Hygiene level.
    /// *   **1.0**: Clean.
    /// *   **< 0.2**: Dirty/Unhappy.
    pub hygiene: f32,
}

impl Default for Needs {
    fn default() -> Self {
        Self {
            hunger: 0.8,
            rest: 0.8,
            leisure: 0.8,
            hygiene: 0.8,
        }
    }
}

impl Needs {
    /// Returns the worst (lowest) need value.
    ///
    /// Used by the AI to determine the most pressing problem.
    ///
    /// # Examples
    ///
    /// ```
    /// use scale::layer1::needs::Needs;
    ///
    /// let needs = Needs { hunger: 0.9, rest: 0.2, leisure: 0.5, hygiene: 1.0 };
    /// assert_eq!(needs.worst(), 0.2); // Rest is the lowest
    /// ```
    #[must_use]
    pub fn get(&self, need_type: NeedType) -> f32 {
        match need_type {
            NeedType::Hunger | NeedType::Food => self.hunger,
            NeedType::Rest => self.rest,
            NeedType::Leisure => self.leisure,
            NeedType::Hygiene => self.hygiene,
        }
    }

    pub fn set(&mut self, need_type: NeedType, value: f32) {
        match need_type {
            NeedType::Hunger | NeedType::Food => self.hunger = value,
            NeedType::Rest => self.rest = value,
            NeedType::Leisure => self.leisure = value,
            NeedType::Hygiene => self.hygiene = value,
        }
    }

    pub const fn worst(&self) -> f32 {
        let min_hr = if self.hunger < self.rest {
            self.hunger
        } else {
            self.rest
        };
        let min_lh = if self.leisure < self.hygiene {
            self.leisure
        } else {
            self.hygiene
        };
        if min_hr < min_lh {
            min_hr
        } else {
            min_lh
        }
    }

    /// Calculates aggregate morale score (0.0 to 1.0).
    ///
    /// Simple average of all needs.
    ///
    /// # Examples
    ///
    /// ```
    /// use scale::layer1::needs::Needs;
    ///
    /// let needs = Needs { hunger: 1.0, rest: 0.5, leisure: 0.0, hygiene: 0.5 };
    /// // (1.0 + 0.5 + 0.0 + 0.5) / 4.0 = 0.5
    /// assert_eq!(needs.morale(), 0.5);
    /// ```
    #[must_use]
    pub fn morale(&self) -> f32 {
        (self.hunger + self.rest + self.leisure + self.hygiene) / 4.0
    }
}

/// Returns work efficiency multiplier based on morale.
///
/// *   **High Morale (>= 0.8)**: 120% Work Speed.
/// *   **Low Morale (<= 0.2)**: 50% Work Speed.
/// *   **Normal**: 100% Work Speed.
#[must_use]
pub fn get_morale_efficiency(morale: f32) -> f32 {
    if morale >= 0.8 {
        1.2
    } else if morale <= 0.2 {
        0.5
    } else {
        1.0
    }
}

/// Applies damage to pops that are starving (hunger <= 0).
pub fn starvation_damage_system(world: &mut World) {
    let tick = world
        .get_resource::<crate::shared::time::SimulationTime>()
        .map_or(0, |t| t.tick);

    let mut query = world.query::<(
        &Needs,
        &mut Health,
        Option<&mut Memories>,
        Option<&crate::layer1::mind::temporal_fugue::TemporalFugue>,
    )>();
    for (needs, mut health, mut memories, fugue) in query.iter_mut(world) {
        if needs.hunger <= 0.0 && fugue.is_none() {
            // Ludwig: Grace Period - Starving should feel urgent but not instant death.
            // 0.2 damage per tick -> 500 ticks (50s) to die.
            health.take_damage(0.2);

            if let Some(mem) = memories.as_mut() {
                mem.add(MemoryType::StarvationTrauma, tick);
            }
        }
    }
}

/// 0.1% decay per tick. Pop starves in ~1000 ticks from full (1.0).
/// (Assuming no traits or rationing).
pub const HUNGER_DECAY_PER_TICK: f32 = 0.001;

/// 0.1% decay per tick. Pop exhausts in ~1000 ticks from rested (1.0).
const REST_DECAY_PER_TICK: f32 = 0.001;

/// 0.15% decay per tick. Slightly faster than physical needs.
const LEISURE_DECAY_PER_TICK: f32 = 0.0015;

/// Decays needs for all pops each tick.
///
/// This system applies the constant decay rates to every entity with a [`Needs`] component.
/// It also checks for:
/// *   **Policies**: Adjusts hunger decay if [`ColonyPolicies`] are active (e.g. Rationing).
/// *   **Traits**: Adjusts hunger decay if the pop has specific [`Traits`] (e.g. Glutton).
///
type DecayNeedsFilter = (
    Without<crate::layer1::cryo::CryoStasis>,
    Without<crate::layer1::somnambulism::Somnambulist>,
);

/// # Threading
/// Uses `par_iter_mut` for parallel processing, as need decay is independent per pop.
pub fn decay_needs_system(
    mut query: Query<
        (
            &mut Needs,
            Option<&Traits>,
            Option<&crate::layer1::temporal_chamber::InsideChamber>,
        ),
        DecayNeedsFilter,
    >,
    chambers: Query<&crate::layer1::temporal_chamber::TemporalChamber>,
    policies: Option<Res<ColonyPolicies>>,
) {
    let hunger_mod = policies.map_or(1.0, |p| get_hunger_decay_modifier(&p));
    let base_hunger_decay = HUNGER_DECAY_PER_TICK * hunger_mod;

    query
        .par_iter_mut()
        .for_each(|(mut needs, traits, inside_chamber)| {
            let hunger_trait_mod = traits.map_or(1.0, get_trait_hunger_decay_modifier);
            let mut hunger_decay = base_hunger_decay * hunger_trait_mod;

            let leisure_trait_mod = traits.map_or(1.0, get_trait_leisure_decay_modifier);
            let mut leisure_decay = LEISURE_DECAY_PER_TICK * leisure_trait_mod;

            let mut rest_decay = REST_DECAY_PER_TICK;

            if traits.is_some_and(|t| t.has(crate::layer1::traits::Trait::InsomniaDrive)) {
                rest_decay = 0.0;
            }

            if let Some(inside) = inside_chamber {
                if let Ok(chamber) = chambers.get(inside.chamber_entity) {
                    if chamber.active {
                        let factor = chamber.time_dilation_factor;
                        hunger_decay *= factor;
                        leisure_decay *= factor;
                        rest_decay *= factor;
                    }
                }
            }

            needs.hunger = (needs.hunger - hunger_decay).max(0.0);
            needs.rest = (needs.rest - rest_decay).max(0.0);
            needs.leisure = (needs.leisure - leisure_decay).max(0.0);
            // Hygiene is decayed separately in hygiene.rs
        });
}

/// System to despawn pops that have reached 0.0 hunger.
///
/// DEPRECATED (revival fix): This system is NOT scheduled. It instantly despawns
/// pops at hunger <= 0.0, which wiped the starting colony on tick 2 before they
/// could eat. Starvation is handled by `starvation_damage_system` (gradual damage).
/// Kept for tests; do not re-add to the schedule without a grace period.
pub fn kill_starving_pops_system(
    mut commands: Commands,
    query: Query<
        (
            Entity,
            &Needs,
            Option<&crate::layer1::mind::temporal_fugue::TemporalFugue>,
        ),
        With<crate::layer1::entities::pop::Pop>,
    >,
) {
    for (entity, needs, fugue) in query.iter() {
        if needs.hunger <= 0.0 && fugue.is_none() {
            commands.entity(entity).despawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::pop::Pop;
    use bevy_ecs::system::RunSystemOnce;

    fn setup() -> World {
        crate::setup::init_task_pools();
        World::new()
    }

    #[test]
    fn test_needs_default() {
        let needs = Needs::default();
        assert!((needs.hunger - 0.8).abs() < f32::EPSILON);
        assert!((needs.rest - 0.8).abs() < f32::EPSILON);
        assert!((needs.hygiene - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn test_needs_worst() {
        let needs1 = Needs {
            hunger: 0.5,
            rest: 0.7,
            leisure: 0.8,
            hygiene: 0.9,
        };
        assert!((needs1.worst() - 0.5).abs() < f32::EPSILON);

        let needs2 = Needs {
            hunger: 0.9,
            rest: 0.3,
            leisure: 0.8,
            hygiene: 0.9,
        };
        assert!((needs2.worst() - 0.3).abs() < f32::EPSILON);

        let needs3 = Needs {
            hunger: 0.5,
            rest: 0.5,
            leisure: 0.5,
            hygiene: 0.1,
        };
        assert!((needs3.worst() - 0.1).abs() < f32::EPSILON);
    }

    #[test]
    fn test_needs_clamped_to_zero() {
        let mut world = setup();
        world.spawn((
            Pop,
            Needs {
                hunger: 0.0001,
                rest: 0.0001,
                leisure: 0.0001,
                hygiene: 0.8,
            },
        ));

        world.run_system_once(decay_needs_system).unwrap();

        let needs = world.query::<&Needs>().single(&world);
        assert!(needs.hunger >= 0.0);
        assert!(needs.rest >= 0.0);
        assert!(needs.hunger < f32::EPSILON);
    }

    #[test]
    fn test_decay_needs_system() {
        let mut world = setup();
        world.spawn((Pop, Needs::default()));

        world.run_system_once(decay_needs_system).unwrap();

        let needs = world.query::<&Needs>().single(&world);
        assert!(needs.hunger < 0.8, "Hunger should have decayed");
        assert!(needs.rest < 0.8, "Rest should have decayed");
        assert!(needs.hunger >= 0.0, "Hunger should not be negative");
        assert!(needs.rest >= 0.0, "Rest should not be negative");
    }

    #[test]
    fn test_decay_multiple_ticks() {
        let mut world = setup();
        world.spawn((Pop, Needs::default()));

        for _ in 0..100 {
            world.run_system_once(decay_needs_system).unwrap();
        }

        let needs = world.query::<&Needs>().single(&world);
        assert!(needs.hunger < 0.71, "Hunger should decay significantly");
        assert!(needs.rest < 0.71, "Rest should decay");
    }

    #[test]
    fn test_calculate_morale() {
        let needs = Needs {
            hunger: 1.0,
            rest: 1.0,
            leisure: 1.0,
            hygiene: 1.0,
        };
        assert!((needs.morale() - 1.0).abs() < f32::EPSILON);

        let needs_mixed = Needs {
            hunger: 0.5,
            rest: 0.5,
            leisure: 0.5,
            hygiene: 0.5,
        };
        assert!((needs_mixed.morale() - 0.5).abs() < f32::EPSILON);

        let needs_bad = Needs {
            hunger: 0.0,
            rest: 0.0,
            leisure: 0.0,
            hygiene: 0.0,
        };
        assert!((needs_bad.morale() - 0.0).abs() < f32::EPSILON);

        // Uneven
        let needs_uneven = Needs {
            hunger: 1.0,
            rest: 0.0,
            leisure: 0.5,
            hygiene: 0.5,
        };
        // (1+0+0.5+0.5)/4 = 2.0/4 = 0.5
        assert!((needs_uneven.morale() - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_morale_efficiency_bonus() {
        // High morale (>= 0.8) -> 1.2x speed
        assert_eq!(super::get_morale_efficiency(0.9), 1.2);
        assert_eq!(super::get_morale_efficiency(0.8), 1.2);
    }

    #[test]
    fn test_morale_efficiency_neutral() {
        // Normal morale (0.2 < m < 0.8) -> 1.0x speed
        assert_eq!(super::get_morale_efficiency(0.5), 1.0);
        assert_eq!(super::get_morale_efficiency(0.79), 1.0);
        assert_eq!(super::get_morale_efficiency(0.21), 1.0);
    }

    #[test]
    fn test_morale_efficiency_penalty() {
        // Low morale (<= 0.2) -> 0.5x speed
        assert_eq!(super::get_morale_efficiency(0.1), 0.5);
        assert_eq!(super::get_morale_efficiency(0.2), 0.5);
    }

    #[test]
    fn test_trait_hunger_decay() {
        use crate::layer1::traits::{Trait, Traits};

        let mut world = setup();
        let glutton = {
            let mut t = Traits::default();
            t.add(Trait::Glutton);
            t
        };

        world.spawn((Pop, Needs::default(), glutton));

        // Initial hunger 0.8
        // Glutton decay = Base (0.001) * 1.2 = 0.0012

        world.run_system_once(decay_needs_system).unwrap();

        let needs = world.query::<&Needs>().single(&world);
        // 0.8 - 0.0012 = 0.7988
        assert!((needs.hunger - 0.7988).abs() < 0.0001);
    }

    #[test]
    fn test_needs_decay_slowed_by_temporal_chamber() {
        use crate::layer1::temporal_chamber::{InsideChamber, TemporalChamber};
        let mut world = setup();

        let chamber = world
            .spawn(TemporalChamber {
                time_dilation_factor: 0.1,
                active: true,
                energy_cost: 10.0,
                ticks_active: 0,
            })
            .id();

        world.spawn((
            Pop,
            Needs::default(),
            InsideChamber {
                chamber_entity: chamber,
                fractional_age: 0.0,
            },
        ));

        world.run_system_once(decay_needs_system).unwrap();

        let needs = world.query::<&Needs>().single(&world);

        // Normal decay is 0.001. Chamber factor is 0.1. So decay is 0.0001.
        // Base 0.8 - 0.0001 = 0.7999
        assert!(
            (needs.hunger - 0.7999).abs() < 0.00001,
            "Hunger decay should be slowed to 10%"
        );
        assert!(
            (needs.rest - 0.7999).abs() < 0.00001,
            "Rest decay should be slowed to 10%"
        );
    }

    #[test]
    fn test_starvation_damage_applied() {
        use crate::layer1::health::Health;
        let mut world = setup();

        let pop = world
            .spawn((
                Pop,
                Needs {
                    hunger: 0.0, // Starving
                    rest: 1.0,
                    leisure: 1.0,
                    hygiene: 1.0,
                },
                Health {
                    current: 100.0,
                    max: 100.0,
                    has_rust_lung: false,
                },
            ))
            .id();

        world.run_system_once(starvation_damage_system).unwrap();

        let health = world.get::<Health>(pop).unwrap();
        // 0.2 damage per tick
        assert!(
            (health.current - 99.8).abs() < f32::EPSILON,
            "Pop should take 0.2 damage when starving"
        );
    }

    #[test]
    fn test_starvation_damage_not_applied_when_not_starving() {
        use crate::layer1::health::Health;
        let mut world = setup();

        let pop = world
            .spawn((
                Pop,
                Needs {
                    hunger: 0.1, // Not starving
                    rest: 1.0,
                    leisure: 1.0,
                    hygiene: 1.0,
                },
                Health {
                    current: 100.0,
                    max: 100.0,
                    has_rust_lung: false,
                },
            ))
            .id();

        world.run_system_once(starvation_damage_system).unwrap();

        let health = world.get::<Health>(pop).unwrap();
        assert!(
            (health.current - 100.0).abs() < f32::EPSILON,
            "Pop should not take damage when hunger is > 0"
        );
    }

    #[test]
    fn test_starvation_adds_trauma_memory() {
        use crate::layer1::health::Health;
        use crate::layer1::memory::{Memories, MemoryType};
        use crate::shared::time::SimulationTime;
        let mut world = setup();
        world.insert_resource(SimulationTime {
            tick: 42,
            ..Default::default()
        });

        let pop = world
            .spawn((
                Pop,
                Needs {
                    hunger: 0.0, // Starving
                    rest: 1.0,
                    leisure: 1.0,
                    hygiene: 1.0,
                },
                Health {
                    current: 100.0,
                    max: 100.0,
                    has_rust_lung: false,
                },
                Memories::default(),
            ))
            .id();

        world.run_system_once(starvation_damage_system).unwrap();

        let memories = world.get::<Memories>(pop).unwrap();
        assert_eq!(memories.items.len(), 1, "A memory should be added");
        let memory = &memories.items[0];
        assert_eq!(memory.memory_type, MemoryType::StarvationTrauma);
        assert_eq!(memory.added_at, 42);
        assert!((memory.intensity - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_insomnia_drive_eliminates_rest_decay() {
        use crate::layer1::traits::{Trait, Traits};

        let mut world = setup();

        let mut traits = Traits::default();
        traits.add(Trait::InsomniaDrive);

        let pop = world
            .spawn((
                Pop,
                Needs {
                    hunger: 1.0,
                    rest: 1.0,
                    leisure: 1.0,
                    hygiene: 1.0,
                },
                traits,
            ))
            .id();

        world.run_system_once(decay_needs_system).unwrap();

        let needs = world.get::<Needs>(pop).unwrap();

        // Hunger should decay normally
        assert!(needs.hunger < 1.0);
        // Rest should not decay at all due to Insomnia Drive
        assert_eq!(needs.rest, 1.0);
    }

    #[test]
    fn test_kill_starving_pops_system() {
        let mut world = setup();

        // Spawn healthy pop
        world.spawn((
            Pop,
            Needs {
                hunger: 0.5,
                rest: 0.5,
                leisure: 0.5,
                hygiene: 0.5,
            },
        ));

        // Spawn starving pop
        world.spawn((
            Pop,
            Needs {
                hunger: 0.0,
                rest: 0.5,
                leisure: 0.5,
                hygiene: 0.5,
            },
        ));

        world.run_system_once(kill_starving_pops_system).unwrap();

        let count = world.query::<&Pop>().iter(&world).count();
        assert_eq!(count, 1, "Only healthy pop should survive");
    }

    #[test]
    fn test_kill_only_when_hunger_zero() {
        let mut world = setup();

        // Pop with very low hunger but not zero
        world.spawn((
            Pop,
            Needs {
                hunger: 0.01,
                rest: 0.0,
                leisure: 0.0,
                hygiene: 0.0,
            },
        ));

        world.run_system_once(kill_starving_pops_system).unwrap();

        let count = world.query::<&Pop>().iter(&world).count();
        assert_eq!(count, 1, "Pop with 0.01 hunger should survive");
    }

    #[test]
    fn test_starve_from_full() {
        let mut world = setup();
        world.spawn((Pop, Needs::default()));

        // Run until pop dies
        let mut ticks = 0;
        while world.query::<&Pop>().iter(&world).count() > 0 && ticks < 1000 {
            world.run_system_once(decay_needs_system).unwrap();
            world.run_system_once(kill_starving_pops_system).unwrap();
            ticks += 1;
        }

        assert!(
            ticks < 850,
            "Pop should die within ~850 ticks from full (0.8)"
        );
        assert!(ticks > 750, "Pop should survive at least 750 ticks");
    }

    // Notice: `pop_display` logic tests as specified by 005.
    // The actual function `get_pop_display` is located in `src/ui/map.rs`.
    // The spec requires testing the display logic, which outputs "☺", "☻", "☹".
    #[test]
    fn test_pop_display_basic_healthy() {
        let needs = Needs {
            hunger: 0.8,
            rest: 0.8,
            leisure: 0.8,
            hygiene: 0.8,
        };
        let (ch, color) = crate::ui::map::get_pop_display(&needs);

        assert_eq!(ch, "☺");
        assert_eq!(color, ratatui::style::Color::Yellow);
    }

    #[test]
    fn test_pop_display_basic_warning() {
        let needs = Needs {
            hunger: 0.5,
            rest: 0.8,
            leisure: 0.8,
            hygiene: 0.8,
        };
        let (ch, color) = crate::ui::map::get_pop_display(&needs);

        assert_eq!(ch, "☻");
        assert_eq!(color, ratatui::style::Color::Rgb(255, 165, 0)); // Orange
    }

    #[test]
    fn test_pop_display_basic_critical() {
        let needs = Needs {
            hunger: 0.2,
            rest: 0.8,
            leisure: 0.8,
            hygiene: 0.8,
        };
        let (ch, color) = crate::ui::map::get_pop_display(&needs);

        assert_eq!(ch, "☹");
        assert_eq!(color, ratatui::style::Color::Red);
    }

    #[test]
    fn test_pop_display_basic_uses_worst_need() {
        // Even if hunger is high, low rest should trigger warning
        let needs = Needs {
            hunger: 0.9,
            rest: 0.4,
            leisure: 0.8,
            hygiene: 0.8,
        };
        let (ch, color) = crate::ui::map::get_pop_display(&needs);

        assert_eq!(ch, "☻"); // Warning state
        assert_eq!(color, ratatui::style::Color::Rgb(255, 165, 0));
    }
}
