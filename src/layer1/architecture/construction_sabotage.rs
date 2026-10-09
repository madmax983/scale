//! Spec 275: Architectural Sabotage.
//!
//! Rival-faction contracted workers can insert "sabotage points" into
//! buildings during construction. A sabotaged building carries a hidden
//! [`Sabotaged`] component with an armed countdown; when the countdown
//! elapses the building fails catastrophically — a rigged explosion
//! (through the existing [`ExplosionEvent`] damage pipeline) collapses the
//! structure and spawns a [`Ruin`](crate::layer1::architecture::ruins::Ruin).
//!
//! Insertion hooks the real construction-completion event
//! ([`BuildingCompletedEvent`]) and scales the sabotage chance inversely
//! with the worst faction-relations score ([`FactionRelations`]), per the
//! spec's REFACTOR phase. The [`VettingPolicy`] resource is the counterplay:
//! Strict vetting blocks all sabotage insertion; Lax doubles the rate.
//!
//! Spec adaptation note: the sketch types `Sabotaged`, `BuildingFailure`
//! and `VolatileExplosion` did not exist. `Sabotaged` is implemented as a
//! real component; the "unified building failure" event is the real
//! [`BuildingFailureEvent`]; damage flows through the real `ExplosionEvent`
//! / `handle_explosion_system` pipeline instead of an invented
//! `VolatileExplosion` API.

use crate::layer1::building::{Building, BuildingType, Material, MaterialType, OccupiedTiles};
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::core::events::{BuildingCompletedEvent, BuildingRemovedEvent};
use crate::layer1::core::map::GridPosition;
use crate::layer1::environment::volatile::ExplosionEvent;
use crate::layer1::ruins::{Ruin, RuinHistory};
use crate::layer3::diplomacy::system_sovereignty::FactionRelations;
use crate::shared::log::MessageLog;
use crate::shared::time::SimulationTime;
use bevy_ecs::prelude::*;
use rand::Rng;

/// Component marking a building whose construction was infiltrated by a
/// rival faction's contracted workers. The countdown is hidden from the
/// colony; when it elapses the building fails catastrophically.
#[derive(Component, Debug, Clone, Copy)]
pub struct Sabotaged {
    /// Ticks remaining before the sabotage points fail.
    pub ticks_until_failure: u32,
    /// The rival faction id blamed for the planting (0 = unknown).
    pub planted_by_faction: u32,
}

/// Unified building-failure event (spec REFACTOR phase): any catastrophic
/// building collapse — sabotage or otherwise — should funnel through this.
#[derive(Event, Debug, Clone, Copy)]
pub struct BuildingFailureEvent {
    /// The building entity that failed.
    pub entity: Entity,
}

/// Internal vetting mode for construction crews.
///
/// This is the counterplay to sabotage: Strict vetting blocks all
/// infiltration, at the cost (flavor only) of slower crew turnover.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VettingMode {
    /// No sabotage points can be planted.
    Strict,
    /// Normal sabotage insertion rate.
    #[default]
    Standard,
    /// Doubled sabotage insertion rate (lax background checks).
    Lax,
}

impl VettingMode {
    /// Short human-readable label, used as the UI indicator.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            VettingMode::Strict => "strict",
            VettingMode::Standard => "standard",
            VettingMode::Lax => "lax",
        }
    }

    /// Parse a mode from a headless console argument.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "strict" => Some(VettingMode::Strict),
            "standard" | "normal" => Some(VettingMode::Standard),
            "lax" | "loose" => Some(VettingMode::Lax),
            _ => None,
        }
    }
}

/// Colony-wide vetting policy for construction crews.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct VettingPolicy {
    /// The active vetting mode.
    pub mode: VettingMode,
}

/// Tuning knobs for the sabotage pipeline. Tests override these to force
/// deterministic outcomes.
#[derive(Resource, Debug, Clone)]
pub struct SabotageDirector {
    /// Sabotage chance at neutral (0) relations, Standard vetting.
    pub base_chance: f64,
    /// Extra chance added at fully hostile (-100) relations.
    pub hostility_multiplier: f64,
    /// Rate multiplier under Lax vetting.
    pub lax_multiplier: f64,
    /// Min ticks before a planted building fails.
    pub min_ticks_to_failure: u32,
    /// Max ticks before a planted building fails.
    pub max_ticks_to_failure: u32,
    /// Damage of the rigged explosion.
    pub explosion_damage: f32,
    /// Radius of the rigged explosion.
    pub explosion_radius: u32,
}

impl Default for SabotageDirector {
    fn default() -> Self {
        Self {
            base_chance: 0.03,
            hostility_multiplier: 0.22,
            lax_multiplier: 2.0,
            min_ticks_to_failure: 800,
            max_ticks_to_failure: 2400,
            explosion_damage: 120.0,
            explosion_radius: 2,
        }
    }
}

/// Pure insertion-rate model: sabotage chance is inverse to faction
/// relations (spec REFACTOR phase). `worst_relations_score` is the minimum
/// of [`FactionRelations`] scores (-100 hostile .. 100 allied).
#[must_use]
pub fn sabotage_chance(
    worst_relations_score: i32,
    mode: VettingMode,
    director: &SabotageDirector,
) -> f64 {
    if mode == VettingMode::Strict {
        return 0.0;
    }
    let hostility = f64::from(worst_relations_score.min(0).unsigned_abs()) / 100.0;
    let mut chance = director.base_chance + director.hostility_multiplier * hostility;
    if mode == VettingMode::Lax {
        chance *= director.lax_multiplier;
    }
    chance.clamp(0.0, 0.95)
}

/// Worst (most hostile) faction-relations score, or 0 when there are no
/// tracked factions.
fn worst_relations_score(relations: Option<&FactionRelations>) -> (i32, u32) {
    let Some(relations) = relations else {
        return (0, 0);
    };
    relations
        .scores
        .iter()
        .min_by_key(|(_, score)| **score)
        .map(|(id, score)| (*score, *id))
        .unwrap_or((0, 0))
}

/// System: on construction completion, rival infiltrators may plant
/// sabotage points. Hooks the real [`BuildingCompletedEvent`].
pub fn insert_sabotage_on_completion_system(
    mut commands: Commands,
    mut completions: EventReader<BuildingCompletedEvent>,
    relations: Option<Res<FactionRelations>>,
    policy: Option<Res<VettingPolicy>>,
    director: Option<Res<SabotageDirector>>,
) {
    let policy = policy.map_or(VettingPolicy::default(), |p| *p);
    let director = director.map_or(SabotageDirector::default(), |d| d.clone());
    if policy.mode == VettingMode::Strict {
        return;
    }
    let (worst_score, worst_faction) = worst_relations_score(relations.as_deref());
    let chance = sabotage_chance(worst_score, policy.mode, &director);
    if chance <= 0.0 {
        return;
    }
    let mut rng = rand::thread_rng();
    for event in completions.read() {
        if rng.gen_bool(chance) {
            let ticks = rng.gen_range(director.min_ticks_to_failure..=director.max_ticks_to_failure);
            if let Some(mut entity_commands) = commands.get_entity(event.entity) {
                entity_commands.insert(Sabotaged {
                    ticks_until_failure: ticks,
                    planted_by_faction: worst_faction,
                });
            }
        }
    }
}

/// System: arm the countdown on every sabotaged building. When it hits
/// zero, emit the unified [`BuildingFailureEvent`].
pub fn tick_sabotage_system(
    mut query: Query<(Entity, &mut Sabotaged)>,
    mut failures: EventWriter<BuildingFailureEvent>,
) {
    for (entity, mut sabotaged) in &mut query {
        if sabotaged.ticks_until_failure > 0 {
            sabotaged.ticks_until_failure -= 1;
        }
        if sabotaged.ticks_until_failure == 0 {
            failures.send(BuildingFailureEvent { entity });
        }
    }
}

/// System: catastrophic failure. Fires the rigged explosion through the
/// existing [`ExplosionEvent`] pipeline (damaging nearby structures and
/// pops), collapses the building into a [`Ruin`](crate::layer1::architecture::ruins::Ruin),
/// emits [`BuildingRemovedEvent`], and writes a Major chronicle entry.
pub fn handle_building_failure_system(world: &mut World) {
    let failed: Vec<Entity> = {
        let mut events = world.resource_mut::<Events<BuildingFailureEvent>>();
        events.drain().map(|e| e.entity).collect()
    };
    if failed.is_empty() {
        return;
    }
    let director = world
        .get_resource::<SabotageDirector>()
        .cloned()
        .unwrap_or_default();
    let tick = world
        .get_resource::<SimulationTime>()
        .map_or(0, |t| t.tick);

    for entity in failed {
        let Ok(entity_ref) = world.get_entity(entity) else {
            continue;
        };
        let Some(pos) = entity_ref.get::<GridPosition>().copied() else {
            continue;
        };
        let building_type = entity_ref
            .get::<Building>()
            .map_or(BuildingType::default(), |b| b.building_type);
        let material = entity_ref
            .get::<Material>()
            .map_or(MaterialType::default(), |m| m.0);

        // The rigged explosion: real damage through the real pipeline.
        world
            .resource_mut::<Events<ExplosionEvent>>()
            .send(ExplosionEvent {
                center: pos,
                damage: director.explosion_damage,
                radius: director.explosion_radius,
            });

        // Collapse the building into a ruin (mirrors the structure-damage
        // destruction path: ruins block the tile, so OccupiedTiles stays).
        world.despawn(entity);
        world.spawn((
            Ruin {
                original_type: building_type,
                material,
            },
            RuinHistory {
                destruction_tick: tick,
                reason: "Sabotage".to_string(),
            },
            pos,
        ));
        world.send_event(BuildingRemovedEvent {
            entity,
            position: pos,
            building_type,
        });

        let label = building_type.label();
        if let Some(mut log) = world.get_resource_mut::<MessageLog>() {
            log.add(format!(
                "SABOTAGE: the {label} at ({}, {}) collapsed in a rigged explosion!",
                pos.x, pos.y
            ));
        }
        world
            .resource_mut::<Events<AddChronicleEvent>>()
            .send(AddChronicleEvent {
                text: format!(
                    "SABOTAGE: the {label} failed catastrophically — rival \
                     infiltrators had rigged it during construction."
                ),
                importance: EventImportance::Major,
            });
        // Keep the tile blocked: a ruin now stands there, same as the
        // structure-damage path.
        let _ = world.get_resource::<OccupiedTiles>();
    }
}

/// Number of currently sabotaged buildings (for STATS / debug).
#[must_use]
pub fn sabotaged_building_count(world: &mut World) -> usize {
    world.query::<&Sabotaged>().iter(world).count()
}

/// Current vetting mode label (UI indicator for headless).
#[must_use]
pub fn vetting_mode_label(world: &World) -> &'static str {
    world
        .get_resource::<VettingPolicy>()
        .map_or(VettingMode::Standard, |p| p.mode)
        .label()
}

/// Debug: detonate every sabotaged building immediately.
pub fn trigger_all_sabotage(world: &mut World) -> usize {
    let targets: Vec<Entity> = world
        .query::<(Entity, &Sabotaged)>()
        .iter(world)
        .map(|(e, _)| e)
        .collect();
    let count = targets.len();
    for entity in targets {
        world.send_event(BuildingFailureEvent { entity });
    }
    count
}

/// Public-facing strings for the IP guard test.
#[must_use]
pub fn sabotage_public_strings() -> Vec<String> {
    vec![
        "SABOTAGE: the collapsed in a rigged explosion!".to_string(),
        "SABOTAGE: the failed catastrophically".to_string(),
        "sabotage".to_string(),
        "vetting".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_world() -> World {
        let mut world = World::new();
        world.insert_resource(FactionRelations::default());
        world.insert_resource(VettingPolicy::default());
        world.insert_resource(SabotageDirector {
            base_chance: 1.0, // force insertion for deterministic tests
            min_ticks_to_failure: 1000,
            max_ticks_to_failure: 1000,
            ..SabotageDirector::default()
        });
        world.init_resource::<Events<BuildingCompletedEvent>>();
        world.init_resource::<Events<BuildingFailureEvent>>();
        world.init_resource::<Events<ExplosionEvent>>();
        world.init_resource::<Events<BuildingRemovedEvent>>();
        world.init_resource::<Events<AddChronicleEvent>>();
        world
    }

    fn spawn_building(world: &mut World, x: i32, y: i32) -> Entity {
        world
            .spawn((
                Building {
                    building_type: BuildingType::Housing,
                },
                GridPosition { x, y },
                crate::layer1::architecture::structure::Structure::default(),
                Material(MaterialType::default()),
            ))
            .id()
    }

    fn run_insertion(world: &mut World) {
        let mut schedule = Schedule::default();
        schedule.add_systems(insert_sabotage_on_completion_system);
        schedule.run(world);
        world.clear_trackers();
    }

    // --- Spec 275 RED phase, adapted to the real types ---

    #[test]
    fn test_sabotage_point_insertion() {
        let mut world = setup_world();
        // Rival faction builder: relations fully hostile.
        world
            .resource_mut::<FactionRelations>()
            .scores
            .insert(7, -100);

        let building = spawn_building(&mut world, 5, 5);
        world
            .resource_mut::<Events<BuildingCompletedEvent>>()
            .send(BuildingCompletedEvent { entity: building });
        run_insertion(&mut world);

        let sabotaged = world
            .get::<Sabotaged>(building)
            .expect("building should carry a Sabotaged component");
        assert_eq!(sabotaged.planted_by_faction, 7);
        assert_eq!(sabotaged.ticks_until_failure, 1000);
    }

    #[test]
    fn test_sabotaged_building_failure() {
        let mut world = setup_world();
        let building = spawn_building(&mut world, 5, 5);
        world.entity_mut(building).insert(Sabotaged {
            ticks_until_failure: 1,
            planted_by_faction: 7,
        });

        // Arm the countdown, then process the failure.
        let mut schedule = Schedule::default();
        schedule.add_systems(tick_sabotage_system);
        schedule.run(&mut world);
        world.clear_trackers();
        handle_building_failure_system(&mut world);

        // Building explodes and collapses: despawned, ruin left behind,
        // explosion event fired into the real damage pipeline.
        assert!(world.get_entity(building).is_err());
        let ruins: Vec<_> = world.query::<&Ruin>().iter(&world).collect();
        assert_eq!(ruins.len(), 1);
        assert_eq!(ruins[0].original_type, BuildingType::Housing);
        let explosions: Vec<ExplosionEvent> = world
            .resource::<Events<ExplosionEvent>>()
            .get_cursor()
            .read(world.resource::<Events<ExplosionEvent>>())
            .cloned()
            .collect();
        assert_eq!(explosions.len(), 1);
        assert_eq!(explosions[0].center, GridPosition { x: 5, y: 5 });
        assert!(explosions[0].damage > 0.0);
        assert!(explosions[0].radius >= 1);
    }

    #[test]
    fn test_vetting_prevents_sabotage() {
        let mut world = setup_world();
        // Strict vetting policy, fully hostile rival builder.
        world.resource_mut::<VettingPolicy>().mode = VettingMode::Strict;
        world
            .resource_mut::<FactionRelations>()
            .scores
            .insert(7, -100);

        let building = spawn_building(&mut world, 5, 5);
        world
            .resource_mut::<Events<BuildingCompletedEvent>>()
            .send(BuildingCompletedEvent { entity: building });
        run_insertion(&mut world);

        assert!(
            world.get::<Sabotaged>(building).is_none(),
            "strict vetting must block all sabotage insertion"
        );
    }

    // --- REFACTOR phase: rate tracks hostility inversely ---

    #[test]
    fn test_sabotage_chance_tracks_hostility() {
        let director = SabotageDirector::default();
        let allied = sabotage_chance(100, VettingMode::Standard, &director);
        let neutral = sabotage_chance(0, VettingMode::Standard, &director);
        let hostile = sabotage_chance(-100, VettingMode::Standard, &director);
        assert!((neutral - director.base_chance).abs() < f64::EPSILON);
        assert!(allied <= neutral, "allied factions plant less");
        assert!(
            (hostile - (director.base_chance + director.hostility_multiplier)).abs() < f64::EPSILON
        );
        assert_eq!(sabotage_chance(-100, VettingMode::Strict, &director), 0.0);
        assert!(
            sabotage_chance(-100, VettingMode::Lax, &director)
                > sabotage_chance(-100, VettingMode::Standard, &director),
            "lax vetting raises the rate"
        );
    }

    #[test]
    fn test_sabotage_countdown_arms_but_does_not_fire_early() {
        let mut world = setup_world();
        let building = spawn_building(&mut world, 5, 5);
        world.entity_mut(building).insert(Sabotaged {
            ticks_until_failure: 10,
            planted_by_faction: 7,
        });
        let mut schedule = Schedule::default();
        schedule.add_systems(tick_sabotage_system);
        schedule.run(&mut world);
        world.clear_trackers();

        // Still standing after one tick, countdown armed.
        assert!(world.get_entity(building).is_ok());
        assert_eq!(world.get::<Sabotaged>(building).unwrap().ticks_until_failure, 9);
        let failures: Vec<BuildingFailureEvent> = world
            .resource::<Events<BuildingFailureEvent>>()
            .get_cursor()
            .read(world.resource::<Events<BuildingFailureEvent>>())
            .cloned()
            .collect();
        assert!(failures.is_empty());
    }

    // --- IP guard ---

    #[test]
    fn test_ip_guard_no_banned_terms() {
        // Architectural Sabotage is concept-only: generic "rival
        // infiltrators" naming, nothing lifted from outside fiction.
        let banned = [
            "emperor", "dune", "arrakis", "atreides", "imperium",
            "sad king billy", "windsor", "windsor-in-exile",
        ];
        for s in sabotage_public_strings() {
            let lower = s.to_lowercase();
            for b in banned {
                assert!(
                    !lower.contains(b),
                    "banned term '{b}' in public string: {s}"
                );
            }
        }
    }
}
