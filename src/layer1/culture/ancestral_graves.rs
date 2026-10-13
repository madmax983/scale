//! Ancestral Graves and Veneration (Spec 349).
//!
//! This module handles the psychological impact of burial sites on the living.
//! Pops who visit nearby graves receive a boost to their leisure/mood needs,
//! simulating ancestral veneration.
//!
//! However, if the colony expands recklessly and builds over an existing grave,
//! it triggers a `SacrilegeEvent`, representing the defilement of sacred ground.
//!
//! # Design decisions (Spec 349 grounding pass, 2026-10-13)
//!
//! * **Graves vs. corpses vs. burial.** The death pipeline
//!   (`handle_pop_death_system`) spawns *both* a physical `Corpse` (the
//!   remains: it decays, composts into soil per Spec 1370 if unburied, or is
//!   despawned by `funeral::bury_corpse` when buried in a built grave plot)
//!   *and* a `Grave` tile at the death site — the ancestral marker this spec
//!   is about. A death-spawned grave is `occupied: true` from birth: it is
//!   that pop's resting place, and marking it unoccupied let the burial
//!   system reuse it for a second corpse and overwrite the deceased's name.
//!   Built `BuildingType::Grave` plots are the opposite: empty (`occupied:
//!   false`) until `bury_corpse` fills them. Both kinds are `Grave` entities
//!   for visit/sacrilege purposes.
//! * **Who is a "relative"?** The sim has no pop-to-pop kinship/family
//!   relation (only offworld `MigrantFamilyInfo`). The honest in-sim proxy
//!   is the social graph: a visitor whose `Relationships` affinity toward
//!   the deceased (recorded on the grave's `AncestralGrave` link at death)
//!   is >= 80 — the codebase's own "Best Friend" band — counts as kin and
//!   receives the stronger "Kin's Vigil" buff. Everyone else is a
//!   colony-mate paying respects and receives the base veneration buff.
//! * **Mood flows through the real morale system.** Visits add a timed
//!   `MoodModifier` to `Morale` (the same mechanism as compost's "Haunted
//!   Soil") instead of only nudging leisure; sacrilege adds a severe,
//!   long-lived negative modifier on top of the existing leisure dip,
//!   unrest spike, and Major chronicle entry (integration bridge).
//! * **Legacy system retired.** The older leisure-only, kin-only
//!   `visit_grave_system` was removed from the schedule and deleted:
//!   `grave_visit_system` subsumes it, and with death graves now
//!   carrying `AncestralGrave` links, running both double-buffed kin.

use crate::layer1::funeral::Grave;
use crate::layer1::map::GridPosition;
use crate::layer1::morale::{MoodModifier, Morale};
use crate::layer1::needs::Needs;
use crate::layer1::pop::Pop;
use crate::layer1::social::Relationships;
use bevy_ecs::prelude::*;

/// Leisure restored by one grave visit (one buff per visit, never stacked
/// per grave — standing in a cemetery is one act of remembrance).
pub const GRAVE_VISIT_LEISURE: f32 = 0.1;
/// Mood buff for a colony-mate visiting a grave.
pub const VENERATION_BUFF: f32 = 0.10;
/// Mood buff for kin (affinity >= [`KIN_AFFINITY_THRESHOLD`]) visiting.
pub const KIN_VIGIL_BUFF: f32 = 0.20;
/// Affinity at/above which a visitor counts as kin of the deceased.
/// Matches the "Best Friend" band documented on `Relationships`.
pub const KIN_AFFINITY_THRESHOLD: f32 = 80.0;
/// Duration (ticks) of the veneration mood modifiers.
pub const VENERATION_DURATION_TICKS: u32 = 300;
/// Mood penalty applied to every pop when a grave is desecrated.
pub const SACRILEGE_MOOD_PENALTY: f32 = -0.25;
/// Duration (ticks) of the sacrilege mood penalty — it lingers.
pub const SACRILEGE_DURATION_TICKS: u32 = 1000;

/// Adds a mood modifier, or refreshes its duration if already present
/// (visits and outrages must not stack duplicates).
fn add_or_refresh_modifier(morale: &mut Morale, label: &str, value: f32, duration: u32) {
    if let Some(existing) = morale.modifiers.iter_mut().find(|m| m.label == label) {
        existing.duration = existing.duration.max(duration);
    } else {
        morale.add_modifier(MoodModifier {
            label: label.to_string(),
            value,
            duration,
        });
    }
}

/// Total number of `Grave` entities in the world (built plots + death
/// markers). Used by the headless `graves` panel and STATS `graves=`.
pub fn grave_count(world: &mut World) -> usize {
    world.query::<&Grave>().iter(world).count()
}

/// Number of graves that hold a named deceased.
pub fn occupied_grave_count(world: &mut World) -> usize {
    world
        .query::<&Grave>()
        .iter(world)
        .filter(|g| g.occupied)
        .count()
}

/// Triggered when a new building is constructed over an existing [`Grave`].
#[derive(Event, Debug, PartialEq)]
pub struct SacrilegeEvent {
    pub pos: GridPosition,
}

/// Restores the leisure need of any [`Pop`] standing adjacent to a [`Grave`].
///
/// Pops find comfort in ancestral veneration. When a pop moves to a tile
/// immediately adjacent to (or on top of) a grave, they receive a mood buff.
///
/// # Examples
/// ```rust
/// use bevy_app::prelude::*;
/// use scale::layer1::culture::ancestral_graves::grave_visit_system;
/// use scale::layer1::funeral::Grave;
/// use scale::layer1::map::GridPosition;
/// use scale::layer1::needs::Needs;
/// use scale::layer1::pop::Pop;
///
/// let mut app = App::new();
/// app.add_systems(Update, grave_visit_system);
///
/// // Spawn a grave at (0, 0)
/// app.world_mut().spawn((Grave::default(), GridPosition { x: 0, y: 0 }));
///
/// // Spawn a sad pop at (1, 0) (adjacent)
/// let pop = app.world_mut().spawn((
///     Pop,
///     Needs { leisure: 0.1, ..Default::default() },
///     GridPosition { x: 1, y: 0 },
/// )).id();
///
/// app.update();
///
/// // The pop visited the grave and gained leisure!
/// let needs = app.world().get::<Needs>(pop).unwrap();
/// assert!(needs.leisure > 0.1);
/// ```
/// Pop-side query for [`grave_visit_system`] (alias keeps clippy's
/// `type_complexity` quiet, per the spec-332/341 house pattern).
pub type GraveVisitorQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static GridPosition,
        &'static mut Needs,
        Option<&'static mut Morale>,
        Option<&'static Relationships>,
    ),
    With<Pop>,
>;

pub fn grave_visit_system(
    mut pops: GraveVisitorQuery,
    graves: Query<(&GridPosition, Option<&AncestralGrave>), With<Grave>>,
) {
    for (pop_pos, mut needs, morale, relationships) in pops.iter_mut() {
        for (grave_pos, link) in graves.iter() {
            if pop_pos.x.abs_diff(grave_pos.x) <= 1 && pop_pos.y.abs_diff(grave_pos.y) <= 1 {
                // One visit = one buff, however many graves surround the pop.
                needs.leisure = (needs.leisure + GRAVE_VISIT_LEISURE).min(1.0);
                if let Some(mut morale) = morale {
                    let is_kin = link.is_some_and(|grave| {
                        relationships.is_some_and(|rels| {
                            rels.get_affinity(grave.original_entity)
                                >= KIN_AFFINITY_THRESHOLD
                        })
                    });
                    if is_kin {
                        add_or_refresh_modifier(
                            &mut morale,
                            "Kin's Vigil",
                            KIN_VIGIL_BUFF,
                            VENERATION_DURATION_TICKS,
                        );
                    } else {
                        add_or_refresh_modifier(
                            &mut morale,
                            "Ancestral Veneration",
                            VENERATION_BUFF,
                            VENERATION_DURATION_TICKS,
                        );
                    }
                }
                break;
            }
        }
    }
}

/// Detects construction over graves and triggers a [`SacrilegeEvent`].
///
/// Listens for [`crate::layer1::core::events::BuildingCompletedEvent`]s. If the newly constructed building shares
/// a [`GridPosition`] with an existing [`Grave`], a `SacrilegeEvent` is emitted.
///
/// # Examples
/// ```rust
/// use bevy_app::prelude::*;
/// use bevy_ecs::prelude::Events;
///
/// use scale::layer1::culture::ancestral_graves::{build_system_wrapper, SacrilegeEvent};
/// use scale::layer1::BuildingCompletedEvent;
/// use scale::layer1::funeral::Grave;
/// use scale::layer1::map::GridPosition;
///
/// let mut app = App::new();
/// app.insert_resource(Events::<BuildingCompletedEvent>::default());
/// app.insert_resource(Events::<SacrilegeEvent>::default());
/// app.add_systems(Update, build_system_wrapper);
///
/// // Spawn a grave at (5, 5)
/// app.world_mut().spawn((Grave::default(), GridPosition { x: 5, y: 5 }));
///
/// // We build a structure exactly on the grave
/// let building = app.world_mut().spawn(GridPosition { x: 5, y: 5 }).id();
/// app.world_mut()
///     .resource_mut::<Events<BuildingCompletedEvent>>()
///     .send(BuildingCompletedEvent { entity: building });
///
/// app.update();
///
/// // Sacrilege!
/// let events = app.world().resource::<Events<SacrilegeEvent>>();
/// assert_eq!(events.len(), 1);
/// ```
pub fn handle_pop_death(world: &mut World, pop_entity: Entity) {
    let pos = *world.get::<GridPosition>(pop_entity).unwrap();
    let name = world
        .get::<crate::layer1::entities::pop::PopName>(pop_entity)
        .unwrap()
        .0
        .clone();

    world.despawn(pop_entity);

    world.spawn((
        Grave {
            // A death grave is occupied from birth — it is this pop's
            // resting place (see module docs). Leaving it unoccupied let
            // the burial system reuse it and overwrite the name.
            occupied: true,
            corpse_name: Some(name),
        },
        pos,
    ));
}

pub fn place_building(
    world: &mut World,
    building: crate::layer1::building::BuildingType,
    pos: GridPosition,
) -> Result<(), &'static str> {
    let mut grave_to_remove = None;
    let mut query = world.query::<(Entity, &GridPosition, &Grave)>();
    for (entity, g_pos, _) in query.iter(world) {
        if *g_pos == pos {
            grave_to_remove = Some(entity);
            break;
        }
    }

    if let Some(e) = grave_to_remove {
        world.despawn(e);
        world
            .resource_mut::<Events<SacrilegeEvent>>()
            .send(SacrilegeEvent { pos });
    }

    world.spawn((
        crate::layer1::architecture::building::Building {
            building_type: building,
        },
        pos,
    ));
    Ok(())
}

pub fn sacrilege_morale_system(
    mut events: EventReader<SacrilegeEvent>,
    mut query: Query<(&mut Needs, Option<&mut Morale>)>,
) {
    for _ in events.read() {
        for (mut needs, morale) in query.iter_mut() {
            needs.leisure = (needs.leisure - 0.1).max(0.0);
            // Spec 349: the penalty is *severe* — a lingering moral wound
            // in the real Morale system, on top of the leisure dip. The
            // unrest spike + Major chronicle live in the integration bridge
            // (`sacrilege_unrest_bridge`).
            if let Some(mut morale) = morale {
                add_or_refresh_modifier(
                    &mut morale,
                    "Sacrilege",
                    SACRILEGE_MOOD_PENALTY,
                    SACRILEGE_DURATION_TICKS,
                );
            }
        }
    }
}

pub fn build_system_wrapper(
    mut events: EventReader<crate::layer1::core::events::BuildingCompletedEvent>,
    mut sacrilege_events: EventWriter<SacrilegeEvent>,
    buildings: Query<&GridPosition>,
    graves: Query<&GridPosition, With<Grave>>,
) {
    for event in events.read() {
        if let Ok(building_pos) = buildings.get(event.entity) {
            for grave_pos in graves.iter() {
                if building_pos.x == grave_pos.x && building_pos.y == grave_pos.y {
                    sacrilege_events.send(SacrilegeEvent { pos: *building_pos });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::system::RunSystemOnce;

    #[test]
    fn test_sacrilege_morale_system() {
        let mut world = World::new();
        world.init_resource::<Events<SacrilegeEvent>>();

        let pop = world
            .spawn(Needs {
                leisure: 1.0,
                ..Default::default()
            })
            .id();

        world
            .resource_mut::<Events<SacrilegeEvent>>()
            .send(SacrilegeEvent {
                pos: GridPosition { x: 0, y: 0 },
            });

        world.run_system_once(sacrilege_morale_system).unwrap();

        let needs = world.get::<Needs>(pop).unwrap();
        assert!(needs.leisure < 1.0);
    }

    #[test]
    fn test_grave_spawns_on_pop_death() {
        let mut world = World::new();
        let pos = GridPosition { x: 5, y: 5 };

        let pop_entity = world
            .spawn((
                Pop,
                pos,
                crate::layer1::entities::pop::PopName("Miner Bob".to_string()),
            ))
            .id();

        // Trigger death logic
        handle_pop_death(&mut world, pop_entity);

        // Verify grave exists at the position
        let graves = world
            .query_filtered::<&GridPosition, With<Grave>>()
            .iter(&world)
            .collect::<Vec<_>>();
        assert_eq!(graves.len(), 1);
        assert_eq!(*graves[0], pos);

        let grave_comp = world.query::<&Grave>().single(&world);
        assert_eq!(grave_comp.corpse_name.as_deref(), Some("Miner Bob"));
    }

    #[test]
    fn test_building_over_grave_causes_sacrilege2() {
        let mut world = World::new();
        world.init_resource::<Events<SacrilegeEvent>>();

        let pos = GridPosition { x: 5, y: 5 };

        let grave_entity = world
            .spawn((
                Grave {
                    occupied: false,
                    corpse_name: Some("Miner Bob".to_string()),
                },
                pos,
            ))
            .id();

        // Attempt to place building over grave
        let result = place_building(
            &mut world,
            crate::layer1::building::BuildingType::Office,
            pos,
        );
        assert!(result.is_ok()); // The building should be allowed

        // The grave should be destroyed or marked overwritten
        assert!(world.get_entity(grave_entity).is_err());

        // Global sacrilege penalty event should be emitted
        let events = world.resource::<Events<SacrilegeEvent>>();
        let cursor = events.get_cursor();
        assert_eq!(cursor.len(events), 1);
    }

    #[test]
    fn test_grave_provides_mood_buff_to_visitor() {
        // Arrange
        let mut world = World::new();
        world.spawn((Grave::default(), GridPosition { x: 0, y: 0 }));
        let visitor = world
            .spawn((
                Pop,
                Needs {
                    leisure: 0.5,
                    ..Default::default()
                },
                GridPosition { x: 1, y: 0 },
            ))
            .id();

        // Act
        let _ = world.run_system_once(grave_visit_system);

        // Assert
        let needs = world.get::<Needs>(visitor).unwrap();
        assert!(
            needs.leisure > 0.5,
            "Visiting grave should restore leisure/mood"
        );
    }

    #[test]
    fn test_building_over_grave_causes_sacrilege() {
        // Arrange
        let mut world = World::new();
        let grave_pos = GridPosition { x: 0, y: 0 };
        world.spawn((Grave::default(), grave_pos));
        let building_entity = world.spawn(grave_pos).id();
        let mut events = Events::<crate::layer1::BuildingCompletedEvent>::default();
        events.send(crate::layer1::BuildingCompletedEvent {
            entity: building_entity,
        });
        world.insert_resource(events);
        world.insert_resource(Events::<SacrilegeEvent>::default());

        // Act
        let _ = world.run_system_once(build_system_wrapper);

        // Assert
        let sacrilege_events = world.get_resource::<Events<SacrilegeEvent>>().unwrap();
        assert_eq!(
            sacrilege_events.len(),
            1,
            "Building over a grave should trigger sacrilege"
        );
    }

    // ---- Spec 349 RED tests (adapted to real types) ----

    #[test]
    fn test_death_grave_is_occupied() {
        // A grave spawned by a death is that pop's resting place: it must be
        // occupied from birth, or the burial system will reuse it for a
        // second corpse and overwrite the deceased's name.
        let mut world = World::new();
        let pop_entity = world
            .spawn((
                Pop,
                GridPosition { x: 3, y: 3 },
                crate::layer1::entities::pop::PopName("Ada".to_string()),
            ))
            .id();

        handle_pop_death(&mut world, pop_entity);

        let grave = world.query::<&Grave>().single(&world);
        assert!(grave.occupied, "death grave must be occupied");
        assert_eq!(grave.corpse_name.as_deref(), Some("Ada"));
    }

    #[test]
    fn test_grave_visit_grants_morale_modifier() {
        // Spec 349: visiting a grave is a *mood* event, not just leisure.
        // It must flow through the real Morale/MoodModifier system.
        let mut world = World::new();
        world.spawn((Grave::default(), GridPosition { x: 0, y: 0 }));
        let visitor = world
            .spawn((
                Pop,
                Needs {
                    leisure: 0.5,
                    ..Default::default()
                },
                crate::layer1::morale::Morale::default(),
                GridPosition { x: 1, y: 0 },
            ))
            .id();

        let _ = world.run_system_once(grave_visit_system);

        let morale = world
            .get::<crate::layer1::morale::Morale>(visitor)
            .unwrap();
        assert!(
            morale
                .modifiers
                .iter()
                .any(|m| m.label == "Ancestral Veneration" && m.value > 0.0),
            "grave visit should add an 'Ancestral Veneration' mood modifier, got {:?}",
            morale.modifiers.iter().map(|m| &m.label).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_grave_visit_kin_bonus_is_stronger() {
        // Spec 349 REFACTOR: visits are for relatives/friends. A pop with a
        // strong bond (affinity >= 80) to the deceased gets a stronger buff
        // than a colony-mate paying respects.
        let mut world = World::new();
        let dead = world.spawn_empty().id();
        world.spawn((
            Grave {
                occupied: true,
                corpse_name: Some("Ada".to_string()),
            },
            AncestralGrave {
                pop_name: "Ada".to_string(),
                original_entity: dead,
            },
            GridPosition { x: 0, y: 0 },
        ));
        let mut rels = Relationships::default();
        rels.set_affinity(dead, 90.0);
        let kin = world
            .spawn((
                Pop,
                Needs::default(),
                crate::layer1::morale::Morale::default(),
                rels,
                GridPosition { x: 1, y: 0 },
            ))
            .id();
        let stranger = world
            .spawn((
                Pop,
                Needs::default(),
                crate::layer1::morale::Morale::default(),
                Relationships::default(),
                GridPosition { x: 0, y: 1 },
            ))
            .id();

        let _ = world.run_system_once(grave_visit_system);

        let kin_best = world
            .get::<crate::layer1::morale::Morale>(kin)
            .unwrap()
            .modifiers
            .iter()
            .map(|m| m.value)
            .fold(0.0f32, f32::max);
        let stranger_best = world
            .get::<crate::layer1::morale::Morale>(stranger)
            .unwrap()
            .modifiers
            .iter()
            .map(|m| m.value)
            .fold(0.0f32, f32::max);
        assert!(
            kin_best > stranger_best,
            "kin buff ({kin_best}) should exceed colony-mate buff ({stranger_best})"
        );
        assert!(kin_best >= 0.2, "kin buff should be at least +0.2, got {kin_best}");
    }

    #[test]
    fn test_grave_visit_does_not_stack_per_grave() {
        // Standing in a cemetery must not multiply the buff per grave.
        let mut world = World::new();
        for (x, y) in [(0, 0), (0, 1), (1, 1)] {
            world.spawn((Grave::default(), GridPosition { x, y }));
        }
        let visitor = world
            .spawn((
                Pop,
                Needs {
                    leisure: 0.5,
                    ..Default::default()
                },
                GridPosition { x: 1, y: 0 },
            ))
            .id();

        let _ = world.run_system_once(grave_visit_system);

        let needs = world.get::<Needs>(visitor).unwrap();
        assert!(
            needs.leisure <= 0.6 + f32::EPSILON,
            "one visit = one buff, got leisure {}",
            needs.leisure
        );
    }

    #[test]
    fn test_sacrilege_adds_severe_morale_modifier() {
        // Spec 349: sacrilege is a *severe* penalty — it must land in the
        // real Morale system, not only as a small leisure dip.
        let mut world = World::new();
        world.init_resource::<Events<SacrilegeEvent>>();
        let pop = world
            .spawn((
                Needs {
                    leisure: 1.0,
                    ..Default::default()
                },
                crate::layer1::morale::Morale::default(),
            ))
            .id();
        world
            .resource_mut::<Events<SacrilegeEvent>>()
            .send(SacrilegeEvent {
                pos: GridPosition { x: 0, y: 0 },
            });

        world.run_system_once(sacrilege_morale_system).unwrap();

        let morale = world.get::<crate::layer1::morale::Morale>(pop).unwrap();
        assert!(
            morale
                .modifiers
                .iter()
                .any(|m| m.label == "Sacrilege" && m.value <= -0.25),
            "sacrilege should add a <= -0.25 'Sacrilege' mood modifier, got {:?}",
            morale
                .modifiers
                .iter()
                .map(|m| (m.label.clone(), m.value))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_grave_count_counts_all_graves() {
        let mut world = World::new();
        assert_eq!(grave_count(&mut world), 0);
        world.spawn((Grave::default(), GridPosition { x: 0, y: 0 }));
        world.spawn((
            Grave {
                occupied: true,
                corpse_name: Some("Ada".to_string()),
            },
            GridPosition { x: 2, y: 2 },
        ));
        assert_eq!(grave_count(&mut world), 2);
        assert_eq!(occupied_grave_count(&mut world), 1);
    }
}

#[derive(Component)]
pub struct AncestralGrave {
    pub pop_name: String,
    pub original_entity: Entity,
}

// NOTE: the legacy `visit_grave_system` (leisure-only, kin-only) was
// retired in Spec 349 — `grave_visit_system` above subsumes it (leisure
// + Morale modifier, kin tier via the same affinity >= 80 check), and
// running both double-buffed kin standing at a death grave.
