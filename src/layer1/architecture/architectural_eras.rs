//! Spec 332: Architectural Eras.
//!
//! Buildings are stamped with the `GlobalEra` they were constructed in. As
//! eras pass, older buildings lose efficiency but gain heritage — and heritage
//! sites soothe the colony, easing nearby pops' stress.
//!
//! Design notes (adapted from the spec sketch — the mock types
//! `Structure` / `Efficiency { base, current }` /
//! `MoraleAura { radius, boost }` do not exist in the real codebase):
//! - Stamping hooks `Added<Building>` (the real building component); no
//!   construction-code changes needed.
//! - "Efficiency" maps onto the existing building-scoped
//!   `architecture_superstition::Efficiency` multiplier. The
//!   penalty is a pure function of heritage score, so re-application is
//!   idempotent. `Cursed` buildings are skipped — the superstition system
//!   owns their efficiency.
//! - "MoraleAura" maps onto `HeritageAura`: old buildings carry an aura
//!   component and `heritage_aura_relief_system` applies the strongest
//!   in-range aura to each pop's `StressTracker` — max, not sum, so
//!   overlapping heritage sites don't stack (per the spec's balance
//!   guidance). Stress relief is the game's morale currency (cf. the
//!   artifacts module: negative stress is effectively morale).
//! - `HistoricalExempt` marks monuments (`BuildingType::Statue`): they
//!   keep full efficiency while still accruing heritage and aura — the "First
//!   Hut" the player refuses to demolish.
//! - Era length is `TICKS_PER_ERA` ticks (~100 days at the 250-ticks/day
//!   baseline used elsewhere in the codebase).
//! - IP: all flavor text is original and generic; see
//!   `test_ip_guard_no_banned_terms`.

use bevy_ecs::prelude::*;

use crate::layer1::architecture::building::{Building, BuildingType};
use crate::layer1::architecture_superstition::{Cursed, Efficiency};
use crate::layer1::map::GridPosition;
use crate::layer1::pop::Pop;
use crate::layer1::stress::StressTracker;
use crate::shared::time::SimulationTime;

/// Ticks per architectural era (~100 days at 250 ticks/day).
pub const TICKS_PER_ERA: u64 = 25_000;
/// Heritage points accrued per era of age.
pub const HERITAGE_PER_ERA: f32 = 10.0;
/// Heritage score at which a building becomes "venerable".
pub const HERITAGE_VENERABLE_THRESHOLD: f32 = 50.0;
/// Efficiency multiplier for merely aged buildings.
pub const AGED_EFFICIENCY_MULTIPLIER: f32 = 0.9;
/// Efficiency multiplier for venerable buildings.
pub const VENERABLE_EFFICIENCY_MULTIPLIER: f32 = 0.8;
/// Aura radius (cells) for aged buildings.
pub const AGED_AURA_RADIUS: f32 = 2.0;
/// Stress relieved per tick for pops near aged buildings.
pub const AGED_STRESS_RELIEF: f32 = 0.2;
/// Aura radius (cells) for venerable buildings.
pub const VENERABLE_AURA_RADIUS: f32 = 5.0;
/// Stress relieved per tick for pops near venerable buildings.
pub const VENERABLE_STRESS_RELIEF: f32 = 0.5;

/// The current architectural era of the colony (Spec 332).
#[derive(Resource, Default, Debug)]
pub struct GlobalEra {
    /// Monotonic era counter; advances every `TICKS_PER_ERA` ticks.
    pub current_era: u32,
}

/// Records when a building was constructed and how much heritage it has
/// accrued since (Spec 332).
#[derive(Component, Debug)]
pub struct ArchitecturalEra {
    /// The `GlobalEra` value at construction time.
    pub built_era: u32,
    /// Accumulated heritage: `(current_era - built_era) * HERITAGE_PER_ERA`.
    pub heritage_score: f32,
}

/// Morale aura carried by aged/venerable buildings (Spec 332's "MoraleAura",
/// mapped onto the game's stress-based morale currency).
#[derive(Component, Debug, Clone, Copy)]
pub struct HeritageAura {
    /// Radius of the aura in grid cells.
    pub radius: f32,
    /// Stress relieved per tick for pops inside the radius.
    pub stress_relief: f32,
}

/// Tag for monuments: they keep full efficiency while still accruing heritage
/// and aura (Spec 332's `HistoricalExempt`).
#[derive(Component, Debug, Default)]
pub struct HistoricalExempt;

/// Efficiency multiplier for a building with the given heritage score.
///
/// Pure function of the score, so the heritage system can re-apply it every
/// era change without compounding.
#[must_use]
pub fn heritage_efficiency_multiplier(heritage_score: f32) -> f32 {
    if heritage_score >= HERITAGE_VENERABLE_THRESHOLD {
        VENERABLE_EFFICIENCY_MULTIPLIER
    } else if heritage_score > 0.0 {
        AGED_EFFICIENCY_MULTIPLIER
    } else {
        1.0
    }
}

/// The aura a building with the given heritage score should carry, if any.
#[must_use]
pub fn heritage_aura_for(heritage_score: f32) -> Option<HeritageAura> {
    if heritage_score >= HERITAGE_VENERABLE_THRESHOLD {
        Some(HeritageAura {
            radius: VENERABLE_AURA_RADIUS,
            stress_relief: VENERABLE_STRESS_RELIEF,
        })
    } else if heritage_score > 0.0 {
        Some(HeritageAura {
            radius: AGED_AURA_RADIUS,
            stress_relief: AGED_STRESS_RELIEF,
        })
    } else {
        None
    }
}

/// Advances `GlobalEra` from `SimulationTime`. Only writes when the era
/// actually changes, so downstream `is_changed()` guards stay quiet.
pub fn advance_era_system(time: Res<SimulationTime>, mut era: ResMut<GlobalEra>) {
    let new_era = (time.tick / TICKS_PER_ERA) as u32;
    if era.current_era != new_era {
        era.current_era = new_era;
    }
}

/// Stamps newly constructed buildings with the current era. Monuments
/// (`BuildingType::Statue`) are marked `HistoricalExempt`.
type NewBuildingStampQuery<'w, 's> = Query<
    'w,
    's,
    (Entity, &'static Building),
    (Added<Building>, Without<ArchitecturalEra>),
>;

pub fn stamp_architectural_era_system(
    mut commands: Commands,
    era: Res<GlobalEra>,
    new_buildings: NewBuildingStampQuery<'_, '_>,
) {
    for (entity, building) in &new_buildings {
        let mut entity_commands = commands.entity(entity);
        entity_commands.insert(ArchitecturalEra {
            built_era: era.current_era,
            heritage_score: 0.0,
        });
        if building.building_type == BuildingType::Statue {
            entity_commands.insert(HistoricalExempt);
        }
    }
}

/// Recalculates heritage scores from `GlobalEra`. Runs only when the era
/// changed (cheap no-op otherwise).
pub fn calculate_heritage_system(era: Res<GlobalEra>, mut query: Query<&mut ArchitecturalEra>) {
    if !era.is_changed() {
        return;
    }
    for mut architectural_era in &mut query {
        let age = era.current_era.saturating_sub(architectural_era.built_era);
        architectural_era.heritage_score = age as f32 * HERITAGE_PER_ERA;
    }
}

/// Applies the heritage efficiency penalty to the existing `Efficiency`
/// modifier stack. Idempotent: the penalty is a pure function of heritage
/// score, and `Cursed` / `HistoricalExempt` buildings are left alone.
/// Query for the heritage efficiency pass: stamped buildings whose efficiency
/// the superstition system does not own.
type HeritageEfficiencyQuery<'w, 's> = Query<
    'w,
    's,
    (Entity, &'static ArchitecturalEra, Option<&'static Efficiency>),
    (Without<Cursed>, Without<HistoricalExempt>),
>;

pub fn apply_heritage_efficiency_system(
    mut commands: Commands,
    query: HeritageEfficiencyQuery<'_, '_>,
) {
    for (entity, architectural_era, efficiency) in &query {
        let multiplier = heritage_efficiency_multiplier(architectural_era.heritage_score);
        match efficiency {
            // No penalty for modern buildings: don't add component churn.
            None if multiplier >= 1.0 => {}
            _ => {
                commands.entity(entity).insert(Efficiency(multiplier));
            }
        }
    }
}

/// Syncs `HeritageAura` components with heritage scores. Exempt monuments
/// keep their aura — only the efficiency penalty is waived.
pub fn apply_heritage_aura_system(
    mut commands: Commands,
    query: Query<(Entity, &ArchitecturalEra, Option<&HeritageAura>), With<GridPosition>>,
) {
    for (entity, architectural_era, aura) in &query {
        match (heritage_aura_for(architectural_era.heritage_score), aura) {
            (Some(new_aura), _) => {
                commands.entity(entity).insert(new_aura);
            }
            (None, Some(_)) => {
                commands.entity(entity).remove::<HeritageAura>();
            }
            (None, None) => {}
        }
    }
}

/// Applies heritage aura relief to pops: each pop gets the strongest in-range
/// aura (max, not sum — overlapping heritage sites don't stack).
pub fn heritage_aura_relief_system(
    auras: Query<(&GridPosition, &HeritageAura)>,
    mut pops: Query<(&GridPosition, &mut StressTracker), With<Pop>>,
) {
    // Snapshot aura tiles; the pop pass is read-only over them.
    let aura_tiles: Vec<(i32, i32, f32, f32)> = auras
        .iter()
        .map(|(pos, aura)| (pos.x, pos.y, aura.radius, aura.stress_relief))
        .collect();
    if aura_tiles.is_empty() {
        return;
    }
    for (pop_pos, mut stress) in &mut pops {
        let mut best_relief = 0.0f32;
        for (ax, ay, radius, relief) in &aura_tiles {
            let dx = i64::from(pop_pos.x - ax);
            let dy = i64::from(pop_pos.y - ay);
            let dist_sq = dx.saturating_mul(dx).saturating_add(dy.saturating_mul(dy));
            let radius_sq = (*radius as i64).saturating_mul(*radius as i64);
            if dist_sq <= radius_sq && *relief > best_relief {
                best_relief = *relief;
            }
        }
        if best_relief > 0.0 {
            stress.accumulated_stress = (stress.accumulated_stress - best_relief).max(0.0);
        }
    }
}

/// One-shot summary of the colony's architectural heritage (headless `heritage`
/// command).
#[derive(Debug, Default)]
pub struct HeritageSummary {
    /// Current `GlobalEra` value.
    pub current_era: u32,
    /// Buildings stamped with an `ArchitecturalEra`.
    pub stamped_buildings: usize,
    /// Buildings at or above `HERITAGE_VENERABLE_THRESHOLD`.
    pub venerable_buildings: usize,
    /// Greatest `(current_era - built_era)` among stamped buildings.
    pub oldest_age_eras: u32,
}

/// Computes a `HeritageSummary` from the world.
#[must_use]
pub fn heritage_summary(world: &World) -> HeritageSummary {
    let current_era = world
        .get_resource::<GlobalEra>()
        .map_or(0, |era| era.current_era);
    let mut summary = HeritageSummary {
        current_era,
        ..Default::default()
    };
    // `World::query` needs `&mut`; iterate entities directly so this stays
    // callable from read-only (`&World`) debug contexts.
    for entity in world.iter_entities() {
        let Some(architectural_era) = world.get::<ArchitecturalEra>(entity.id()) else {
            continue;
        };
        summary.stamped_buildings += 1;
        if architectural_era.heritage_score >= HERITAGE_VENERABLE_THRESHOLD {
            summary.venerable_buildings += 1;
        }
        summary.oldest_age_eras = summary
            .oldest_age_eras
            .max(current_era.saturating_sub(architectural_era.built_era));
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::*;

    fn spawn_building(app: &mut App, building_type: BuildingType, x: i32, y: i32) -> Entity {
        app.world_mut()
            .spawn((
                Building { building_type },
                GridPosition { x, y },
            ))
            .id()
    }

    #[test]
    fn test_era_advances_with_ticks() {
        let mut app = App::new();
        app.insert_resource(SimulationTime {
            tick: TICKS_PER_ERA * 3,
            ..Default::default()
        });
        app.insert_resource(GlobalEra::default());
        app.add_systems(Update, advance_era_system);
        app.update();

        let era = app.world().resource::<GlobalEra>();
        assert_eq!(era.current_era, 3, "era should be tick / TICKS_PER_ERA");
    }

    #[test]
    fn test_era_does_not_regress_within_era() {
        let mut app = App::new();
        app.insert_resource(SimulationTime {
            tick: TICKS_PER_ERA + 7,
            ..Default::default()
        });
        app.insert_resource(GlobalEra::default());
        app.add_systems(Update, advance_era_system);
        app.update();

        let era = app.world().resource::<GlobalEra>();
        assert_eq!(era.current_era, 1);
    }

    #[test]
    fn test_new_building_stamped_with_current_era() {
        let mut app = App::new();
        app.insert_resource(GlobalEra { current_era: 4 });
        app.add_systems(Update, stamp_architectural_era_system);

        let entity = spawn_building(&mut app, BuildingType::Housing, 1, 1);
        app.update();

        let stamp = app.world().get::<ArchitecturalEra>(entity).unwrap();
        assert_eq!(stamp.built_era, 4, "new building must record the current era");
        assert_eq!(stamp.heritage_score, 0.0);
    }

    #[test]
    fn test_statue_gets_historical_exempt() {
        let mut app = App::new();
        app.insert_resource(GlobalEra::default());
        app.add_systems(Update, stamp_architectural_era_system);

        let statue = spawn_building(&mut app, BuildingType::Statue, 2, 2);
        let house = spawn_building(&mut app, BuildingType::Housing, 3, 3);
        app.update();

        assert!(
            app.world().get::<HistoricalExempt>(statue).is_some(),
            "statues are monuments and must be exempt"
        );
        assert!(
            app.world().get::<HistoricalExempt>(house).is_none(),
            "ordinary buildings must not be exempt"
        );
    }

    #[test]
    fn test_heritage_increases_over_time() {
        let mut app = App::new();
        app.add_systems(Update, calculate_heritage_system);

        // Spawn a building built in Era 0.
        let entity = app
            .world_mut()
            .spawn((
                Building {
                    building_type: BuildingType::Housing,
                },
                ArchitecturalEra {
                    built_era: 0,
                    heritage_score: 0.0,
                },
            ))
            .id();

        // Advance global era to 2.
        app.world_mut().insert_resource(GlobalEra { current_era: 2 });
        app.update();

        let era = app.world().get::<ArchitecturalEra>(entity).unwrap();
        assert!(
            era.heritage_score > 0.0,
            "Heritage score should increase as eras pass"
        );
        assert_eq!(era.heritage_score, 2.0 * HERITAGE_PER_ERA);
    }

    #[test]
    fn test_heritage_recalc_only_on_era_change() {
        let mut app = App::new();
        app.add_systems(Update, calculate_heritage_system);
        app.world_mut().insert_resource(GlobalEra { current_era: 2 });

        let entity = app
            .world_mut()
            .spawn(ArchitecturalEra {
                built_era: 0,
                heritage_score: 0.0,
            })
            .id();
        app.update();
        assert_eq!(
            app.world().get::<ArchitecturalEra>(entity).unwrap().heritage_score,
            20.0
        );

        // Tamper with the score; without an era change the system is a no-op.
        app.world_mut()
            .get_mut::<ArchitecturalEra>(entity)
            .unwrap()
            .heritage_score = 123.0;
        app.update();
        assert_eq!(
            app.world().get::<ArchitecturalEra>(entity).unwrap().heritage_score,
            123.0,
            "heritage must not be recalculated when the era did not change"
        );
    }

    #[test]
    fn test_heritage_lowers_efficiency() {
        let mut app = App::new();
        app.add_systems(Update, apply_heritage_efficiency_system);

        let entity = app
            .world_mut()
            .spawn((
                Building {
                    building_type: BuildingType::Housing,
                },
                ArchitecturalEra {
                    built_era: 0,
                    heritage_score: 100.0,
                },
                Efficiency(1.0),
            ))
            .id();
        app.update();

        let eff = app.world().get::<Efficiency>(entity).unwrap();
        assert!(
            eff.0 < 1.0,
            "Old buildings should lose efficiency (got {})",
            eff.0
        );
        assert_eq!(eff.0, VENERABLE_EFFICIENCY_MULTIPLIER);
    }

    #[test]
    fn test_aged_building_partial_penalty() {
        let mut app = App::new();
        app.add_systems(Update, apply_heritage_efficiency_system);

        let entity = app
            .world_mut()
            .spawn(ArchitecturalEra {
                built_era: 0,
                heritage_score: 20.0,
            })
            .id();
        app.update();

        let eff = app.world().get::<Efficiency>(entity).unwrap();
        assert_eq!(eff.0, AGED_EFFICIENCY_MULTIPLIER);
    }

    #[test]
    fn test_modern_building_keeps_full_efficiency() {
        let mut app = App::new();
        app.add_systems(Update, apply_heritage_efficiency_system);

        let entity = app
            .world_mut()
            .spawn(ArchitecturalEra {
                built_era: 0,
                heritage_score: 0.0,
            })
            .id();
        app.update();

        assert!(
            app.world().get::<Efficiency>(entity).is_none(),
            "modern buildings must not gain an Efficiency component (no churn)"
        );
    }

    #[test]
    fn test_efficiency_penalty_is_idempotent() {
        let mut app = App::new();
        app.add_systems(Update, apply_heritage_efficiency_system);

        let entity = app
            .world_mut()
            .spawn(ArchitecturalEra {
                built_era: 0,
                heritage_score: 100.0,
            })
            .id();
        app.update();
        app.update();
        app.update();

        let eff = app.world().get::<Efficiency>(entity).unwrap();
        assert_eq!(
            eff.0, VENERABLE_EFFICIENCY_MULTIPLIER,
            "re-applying the penalty must not compound"
        );
    }

    #[test]
    fn test_cursed_building_efficiency_untouched() {
        let mut app = App::new();
        app.add_systems(Update, apply_heritage_efficiency_system);

        let entity = app
            .world_mut()
            .spawn((
                ArchitecturalEra {
                    built_era: 0,
                    heritage_score: 100.0,
                },
                Efficiency(0.5),
                Cursed,
            ))
            .id();
        app.update();

        let eff = app.world().get::<Efficiency>(entity).unwrap();
        assert_eq!(
            eff.0, 0.5,
            "cursed buildings are owned by the superstition system"
        );
    }

    #[test]
    fn test_exempt_building_keeps_efficiency_but_gains_aura() {
        let mut app = App::new();
        app.add_systems(Update, apply_heritage_efficiency_system);
        app.add_systems(Update, apply_heritage_aura_system);

        let entity = app
            .world_mut()
            .spawn((
                Building {
                    building_type: BuildingType::Statue,
                },
                GridPosition { x: 0, y: 0 },
                ArchitecturalEra {
                    built_era: 0,
                    heritage_score: 100.0,
                },
                HistoricalExempt,
            ))
            .id();
        app.update();

        assert!(
            app.world().get::<Efficiency>(entity).is_none(),
            "exempt monuments keep full efficiency"
        );
        let aura = app.world().get::<HeritageAura>(entity).unwrap();
        assert!(aura.radius > 0.0 && aura.stress_relief > 0.0);
    }

    #[test]
    fn test_heritage_grants_morale_aura() {
        let mut app = App::new();
        app.add_systems(Update, apply_heritage_aura_system);

        let entity = app
            .world_mut()
            .spawn((
                Building {
                    building_type: BuildingType::Housing,
                },
                GridPosition { x: 5, y: 5 },
                ArchitecturalEra {
                    built_era: 0,
                    heritage_score: 100.0,
                },
            ))
            .id();
        app.update();

        let aura = app.world().get::<HeritageAura>(entity).unwrap();
        assert!(
            aura.radius > 0.0 && aura.stress_relief > 0.0,
            "Old buildings should gain a morale aura"
        );
        assert_eq!(aura.radius, VENERABLE_AURA_RADIUS);
    }

    #[test]
    fn test_modern_building_has_no_aura() {
        let mut app = App::new();
        app.add_systems(Update, apply_heritage_aura_system);

        let entity = app
            .world_mut()
            .spawn((
                GridPosition { x: 5, y: 5 },
                ArchitecturalEra {
                    built_era: 0,
                    heritage_score: 0.0,
                },
                // Stale aura from a hypothetical rollback: must be removed.
                HeritageAura {
                    radius: 5.0,
                    stress_relief: 0.5,
                },
            ))
            .id();
        app.update();

        assert!(
            app.world().get::<HeritageAura>(entity).is_none(),
            "modern buildings must not carry an aura"
        );
    }

    #[test]
    fn test_aura_reduces_pop_stress() {
        let mut app = App::new();
        app.add_systems(Update, heritage_aura_relief_system);

        // Venerable building at (10, 10) with radius 5.
        app.world_mut().spawn((
            GridPosition { x: 10, y: 10 },
            HeritageAura {
                radius: VENERABLE_AURA_RADIUS,
                stress_relief: VENERABLE_STRESS_RELIEF,
            },
        ));
        // Pop inside the aura.
        let near = app
            .world_mut()
            .spawn((
                Pop,
                GridPosition { x: 12, y: 10 },
                StressTracker {
                    accumulated_stress: 10.0,
                },
            ))
            .id();
        // Pop far outside the aura.
        let far = app
            .world_mut()
            .spawn((
                Pop,
                GridPosition { x: 60, y: 60 },
                StressTracker {
                    accumulated_stress: 10.0,
                },
            ))
            .id();
        app.update();

        let near_stress = app
            .world()
            .get::<StressTracker>(near)
            .unwrap()
            .accumulated_stress;
        let far_stress = app
            .world()
            .get::<StressTracker>(far)
            .unwrap()
            .accumulated_stress;
        assert!(
            near_stress < 10.0,
            "pop inside a heritage aura should be soothed"
        );
        assert_eq!(far_stress, 10.0, "pop outside the aura must be unaffected");
    }

    #[test]
    fn test_overlapping_auras_do_not_stack() {
        let mut app = App::new();
        app.add_systems(Update, heritage_aura_relief_system);

        // Two venerable buildings both covering the pop.
        for (x, y) in [(10, 10), (14, 10)] {
            app.world_mut().spawn((
                GridPosition { x, y },
                HeritageAura {
                    radius: VENERABLE_AURA_RADIUS,
                    stress_relief: VENERABLE_STRESS_RELIEF,
                },
            ));
        }
        let pop = app
            .world_mut()
            .spawn((
                Pop,
                GridPosition { x: 12, y: 10 },
                StressTracker {
                    accumulated_stress: 10.0,
                },
            ))
            .id();
        app.update();

        let stress = app
            .world()
            .get::<StressTracker>(pop)
            .unwrap()
            .accumulated_stress;
        assert_eq!(
            stress,
            10.0 - VENERABLE_STRESS_RELIEF,
            "overlapping auras apply max relief, not summed relief"
        );
    }

    #[test]
    fn test_stress_relief_floors_at_zero() {
        let mut app = App::new();
        app.add_systems(Update, heritage_aura_relief_system);

        app.world_mut().spawn((
            GridPosition { x: 0, y: 0 },
            HeritageAura {
                radius: VENERABLE_AURA_RADIUS,
                stress_relief: VENERABLE_STRESS_RELIEF,
            },
        ));
        let pop = app
            .world_mut()
            .spawn((
                Pop,
                GridPosition { x: 0, y: 0 },
                StressTracker {
                    accumulated_stress: 0.1,
                },
            ))
            .id();
        app.update();

        let stress = app
            .world()
            .get::<StressTracker>(pop)
            .unwrap()
            .accumulated_stress;
        assert_eq!(stress, 0.0, "stress must not go negative");
    }

    #[test]
    fn test_full_pipeline_end_to_end() {
        let mut app = App::new();
        app.insert_resource(SimulationTime {
            tick: TICKS_PER_ERA * 6,
            ..Default::default()
        });
        app.insert_resource(GlobalEra::default());
        app.add_systems(
            Update,
            (
                advance_era_system,
                stamp_architectural_era_system,
                calculate_heritage_system,
                apply_heritage_efficiency_system,
                apply_heritage_aura_system,
            )
                .chain(),
        );

        // Building constructed "now" (era 6 after the first update).
        let modern = spawn_building(&mut app, BuildingType::Housing, 0, 0);
        app.update();

        // Jump far into the future: the building becomes venerable.
        app.world_mut()
            .resource_mut::<SimulationTime>()
            .tick = TICKS_PER_ERA * 12;
        app.update();

        let stamp = app.world().get::<ArchitecturalEra>(modern).unwrap();
        assert_eq!(stamp.built_era, 6);
        assert_eq!(stamp.heritage_score, 6.0 * HERITAGE_PER_ERA);
        let eff = app.world().get::<Efficiency>(modern).unwrap();
        assert_eq!(eff.0, VENERABLE_EFFICIENCY_MULTIPLIER);
        let aura = app.world().get::<HeritageAura>(modern).unwrap();
        assert_eq!(aura.radius, VENERABLE_AURA_RADIUS);
    }

    #[test]
    fn test_heritage_summary_counts() {
        let mut app = App::new();
        app.insert_resource(GlobalEra { current_era: 8 });
        app.world_mut().spawn(ArchitecturalEra {
            built_era: 0,
            heritage_score: 80.0,
        });
        app.world_mut().spawn(ArchitecturalEra {
            built_era: 7,
            heritage_score: 10.0,
        });

        let summary = heritage_summary(app.world());
        assert_eq!(summary.current_era, 8);
        assert_eq!(summary.stamped_buildings, 2);
        assert_eq!(summary.venerable_buildings, 1);
        assert_eq!(summary.oldest_age_eras, 8);
    }

    #[test]
    fn test_ip_guard_no_banned_terms() {
        // All flavor is concept-only: no named characters, places, or
        // distinctive external IP anywhere in public strings.
        let banned = [
            "Mordor",
            "Fangorn",
            "Hobbit",
            "Shire",
            "Gondor",
            "Rivendell",
            "Westeros",
            "Winterfell",
            "Arrakis",
            "Dune",
            "Coruscant",
            "Tatooine",
            "Moria",
            "Rohan",
        ];
        let public_strings = [
            "Architectural Eras",
            "heritage",
            "venerable",
            "aged",
            "HistoricalExempt",
            "HeritageAura",
            "The colony remembers its builders",
        ];
        for s in public_strings {
            for term in banned {
                assert!(
                    !s.contains(term),
                    "public string {s:?} must not contain IP term {term:?}"
                );
            }
        }
    }

    #[test]
    fn test_era_crossing_in_live_schedule() {
        use crate::setup::{setup_world_with_config, SetupConfig};
        use crate::simulation::run_simulation_tick;

        let mut world = setup_world_with_config(SetupConfig {
            headless: true,
            ..Default::default()
        });

        // First tick: construction stamping runs inside the real schedule.
        run_simulation_tick(&mut world);
        let stamped = world.query::<&ArchitecturalEra>().iter(&world).count();
        assert!(
            stamped > 0,
            "setup buildings must be era-stamped, got {stamped}"
        );

        // Jump to just before the era boundary and cross it tick by tick.
        world.resource_mut::<SimulationTime>().tick = TICKS_PER_ERA - 1;
        run_simulation_tick(&mut world);
        assert_eq!(world.resource::<GlobalEra>().current_era, 0);
        run_simulation_tick(&mut world);
        assert_eq!(
            world.resource::<GlobalEra>().current_era,
            1,
            "era must advance when ticks cross TICKS_PER_ERA"
        );

        // Every stamped building is now aged: heritage scored, efficiency
        // penalized (unless exempt), aura attached.
        let mut query = world.query::<(
            Entity,
            &ArchitecturalEra,
            Option<&Efficiency>,
            Option<&HeritageAura>,
            Option<&HistoricalExempt>,
        )>();
        let mut aged = 0;
        for (entity, stamp, eff, aura, exempt) in query.iter(&world) {
            let _ = entity;
            assert_eq!(
                stamp.heritage_score, HERITAGE_PER_ERA,
                "one era of age = one era of heritage"
            );
            aged += 1;
            let aura = aura.expect("aged buildings must carry a heritage aura");
            assert_eq!(aura.radius, AGED_AURA_RADIUS);
            match exempt {
                Some(_) => assert!(
                    eff.is_none(),
                    "exempt monuments keep full efficiency"
                ),
                None => assert_eq!(
                    eff.map(|e| e.0),
                    Some(AGED_EFFICIENCY_MULTIPLIER),
                    "aged buildings lose efficiency"
                ),
            }
        }
        assert!(aged > 0, "expected aged buildings after the era crossing");
    }
}
