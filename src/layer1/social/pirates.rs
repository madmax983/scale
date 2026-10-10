use crate::layer1::core::integration::PirateAmnestyEvent;
use crate::layer1::economy::Wallet;
use crate::layer1::map::GridPosition;
use crate::layer1::pop::Pop;
use crate::layer1::traits::{Trait, Traits};
use bevy::prelude::*;

use crate::layer1::law::justice::{CrimeCommittedEvent, CrimeType};
use crate::layer3::physics::relativity::SimulationTime as RelativitySimulationTime;
use rand::Rng;

use crate::layer1::combat::{AttackProperties, Weapon};
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::economy::resources::{ColonyResources, ResourceType};
use crate::layer1::entities::pop::Speed;
use crate::layer1::private_stash::PrivateStash;
use crate::layer1::psychology::needs::Needs;
use crate::layer1::social::generational_dissonance::EdictCompliance;
use crate::layer1::social::morale::{MoodModifier, Morale};
use crate::layer1::unrest::{Unrest, UnrestModifier};
use crate::shared::time::SimulationTime;

// --- Spec 322: The Pirate's Pension ----------------------------------------

/// Pirates landed per amnestied fleet.
pub const PIRATES_PER_FLEET: u32 = 5;
/// Veteran cutlass damage. Normal migrants are unarmed (0.0 damage), so
/// veterans are "significantly higher combat" by construction.
pub const VETERAN_CUTLASS_DAMAGE: f32 = 25.0;
/// Veteran move-speed multiplier (normal migrants: 1.0) — the athletics edge.
pub const VETERAN_SPEED: f32 = 1.3;
/// Pension upkeep: credits per living pirate pop per period.
pub const PENSION_PER_PIRATE: f32 = 50.0;
/// Ticks between pension payouts.
pub const PENSION_PERIOD_TICKS: u64 = 1000;
/// Unrest added by a pension default (feeds the real unrest mechanism).
pub const PENSION_DEFAULT_UNREST: f32 = 0.35;
/// Duration (ticks) of a default's morale/unrest fallout.
pub const PENSION_DEFAULT_DURATION: u32 = 2000;
/// Base per-tick theft probability, scaled by morale/hunger deficits.
pub const THEFT_BASE_PROBABILITY: f64 = 0.002;

/// Fired when the colony treasury cannot cover the pirate pension upkeep
/// (spec §5 REFACTOR: pension defaulting as a real event).
#[derive(Event, Debug, Clone)]
pub struct PensionDefaultEvent {
    /// Living pirate pops the colony failed to pay.
    pub pirate_count: u32,
    /// Credits short of the full upkeep cost.
    pub shortfall: f32,
}

/// Spawns Layer 1 Pirate pops in response to a Layer 3 pirate fleet accepting amnesty.
pub fn process_pirate_amnesty_system(
    mut commands: Commands,
    mut amnesty_events: EventReader<PirateAmnestyEvent>,
    pop_positions: Query<&GridPosition, With<Pop>>,
) {
    use rand::seq::SliceRandom;
    let mut rng = rand::thread_rng();
    // Amnestied pirates join the colony proper: land on a living pop's
    // tile so they arrive inside the pressurized habitat, not at (0, 0).
    let colony_tiles: Vec<GridPosition> = pop_positions.iter().copied().collect();
    for _ev in amnesty_events.read() {
        // Spawn multiple pirate pops per fleet
        for _ in 0..PIRATES_PER_FLEET {
            let mut pirate_traits = Traits::default();
            pirate_traits.add(Trait::Pirate);

            // GHOST-POP FIX (2026-10-04): amnestied pirates used to spawn
            // as bare Pop+Traits+Wallet — no Health, no PopName, no
            // GridPosition — invisible to STATS and immune to damage.
            // Spawn a full PopBundle instead, keeping the Pirate trait and
            // the amnesty credit boost.
            let spawn_pos = colony_tiles
                .choose(&mut rng)
                .copied()
                .unwrap_or(GridPosition { x: 0, y: 0 });
            let mut bundle =
                crate::layer1::PopBundle::random(spawn_pos.x, spawn_pos.y, &mut rng);
            bundle.traits = pirate_traits;
            bundle.wallet = Wallet { credits: 1000.0 }; // Massive credit boost

            // SPEC 322 — veterans, not deckhands.
            //
            // Elite combat: normal migrants are unarmed (0.0 damage);
            // veterans keep their cutlasses.
            let cutlass = commands
                .spawn(Weapon {
                    properties: AttackProperties {
                        damage: VETERAN_CUTLASS_DAMAGE,
                        range: 1.0,
                        cooldown: 8,
                        accuracy: 1.0,
                    },
                })
                .id();
            bundle.equipment.weapon = Some(cutlass);
            // Elite athletics: 30% faster than a normal migrant.
            bundle.speed = Speed {
                base: VETERAN_SPEED,
                current: VETERAN_SPEED,
                accumulator: 0.0,
            };
            // Law-ignoring: veterans disregard safety edicts.
            bundle.edict_compliance = EdictCompliance {
                ignores_safety: true,
            };

            let pirate = commands.spawn(bundle).id();
            // Every veteran gets a PrivateStash for the theft system below.
            commands.entity(pirate).insert(PrivateStash {
                owner: Some(pirate),
                ..Default::default()
            });
        }
    }
}

/// Pirates frequently shirk work and engage in brawls/crime randomly.
pub fn pirate_crime_system(
    mut crime_events: EventWriter<CrimeCommittedEvent>,
    pirates: Query<(Entity, &Traits), With<Pop>>,
    _sim_time: Res<RelativitySimulationTime>,
) {
    let mut rng = rand::thread_rng();

    for (entity, traits) in pirates.iter() {
        if traits.has(Trait::Pirate) {
            // Determine base on arbitrary periodic check
            // e.g. 5% chance per tick to commit vandalism
            if rng.gen_bool(0.05) {
                crime_events.send(CrimeCommittedEvent {
                    perpetrator: entity,
                    crime_type: CrimeType::Vandalism,
                });
            }
        }
    }
}

/// Per-tick theft probability for a pirate pop.
///
/// Morale/needs-driven (spec §5 REFACTOR): a content, well-fed pirate rarely
/// steals; a miserable, starving one steals far more often. Never steals
/// every tick.
#[must_use]
pub fn pirate_theft_probability(morale: f32, hunger: f32) -> f64 {
    let morale_deficit = f64::from(1.0 - morale.clamp(0.0, 1.0));
    let hunger_deficit = f64::from(1.0 - hunger.clamp(0.0, 1.0));
    THEFT_BASE_PROBABILITY + morale_deficit * 0.004 + hunger_deficit * 0.004
}

/// Pirates probabilistically steal food from the colony stockpile into their
/// PrivateStash. Each theft is reported to the Justice System as a real
/// [`CrimeType::Theft`] so wardens can react (spec §7 technical guidance —
/// the crime event is the justice hook; no new ActionType was added to the
/// utility AI, which would have touched the hot evaluation path).
pub fn pirate_theft_system(
    mut crime_events: EventWriter<CrimeCommittedEvent>,
    mut pirates: Query<(Entity, &Traits, &Morale, &Needs, &mut PrivateStash), With<Pop>>,
    mut resources: ResMut<ColonyResources>,
) {
    let mut rng = rand::thread_rng();
    for (entity, traits, morale, needs, mut stash) in pirates.iter_mut() {
        if !traits.has(Trait::Pirate) {
            continue;
        }
        if resources.food <= 0.0 {
            continue;
        }
        if !rng.gen_bool(pirate_theft_probability(morale.value, needs.hunger)) {
            continue;
        }
        let amount = rng.gen_range(2.0f32..8.0).min(resources.food);
        resources.food -= amount;
        stash.add(ResourceType::Food, amount);
        crime_events.send(CrimeCommittedEvent {
            perpetrator: entity,
            crime_type: CrimeType::Theft,
        });
    }
}

/// Amnestied pirates brawl: low morale drives fights, reported as
/// [`CrimeType::Assault`] so the Justice System reacts. (The codebase has no
/// Aggression need, so the justice lane — not a utility-AI Fight threshold —
/// is the real brawl mechanism.)
pub fn pirate_brawl_system(
    mut crime_events: EventWriter<CrimeCommittedEvent>,
    pirates: Query<(Entity, &Traits, &Morale), With<Pop>>,
) {
    let mut rng = rand::thread_rng();
    for (entity, traits, morale) in pirates.iter() {
        if !traits.has(Trait::Pirate) {
            continue;
        }
        let morale_deficit = f64::from(1.0 - morale.value.clamp(0.0, 1.0));
        if rng.gen_bool(0.001 + morale_deficit * 0.004) {
            crime_events.send(CrimeCommittedEvent {
                perpetrator: entity,
                crime_type: CrimeType::Assault,
            });
        }
    }
}

/// Charges the colony a pension upkeep per living pirate pop every
/// [`PENSION_PERIOD_TICKS`] ticks. On default the treasury is drained to
/// zero and a [`PensionDefaultEvent`] fires (spec §5 REFACTOR).
pub fn process_pension_upkeep_system(
    mut resources: ResMut<ColonyResources>,
    pirates: Query<&Traits, With<Pop>>,
    time: Option<Res<SimulationTime>>,
    mut default_events: EventWriter<PensionDefaultEvent>,
) {
    let Some(time) = time else { return };
    if time.tick == 0 || !time.tick.is_multiple_of(PENSION_PERIOD_TICKS) {
        return;
    }
    let pirate_count = pirates
        .iter()
        .filter(|t| t.has(Trait::Pirate))
        .count() as u32;
    if pirate_count == 0 {
        return;
    }
    #[allow(clippy::cast_precision_loss)]
    let cost = (pirate_count as f32) * PENSION_PER_PIRATE;
    if resources.credits >= cost {
        resources.credits -= cost;
    } else {
        let shortfall = cost - resources.credits;
        resources.credits = 0.0;
        default_events.send(PensionDefaultEvent {
            pirate_count,
            shortfall,
        });
    }
}

/// A pension default tanks every pirate's morale and feeds the real colony
/// Unrest mechanism (spec §5 REFACTOR).
pub fn process_pension_default_system(
    mut events: EventReader<PensionDefaultEvent>,
    mut pirates: Query<(&Traits, &mut Morale), With<Pop>>,
    mut unrest: ResMut<Unrest>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    for ev in events.read() {
        for (traits, mut morale) in pirates.iter_mut() {
            if traits.has(Trait::Pirate) {
                morale.add_modifier(MoodModifier {
                    label: "Pension Defaulted".to_string(),
                    value: -0.5,
                    duration: PENSION_DEFAULT_DURATION,
                });
            }
        }
        unrest.modifiers.push(UnrestModifier {
            value: PENSION_DEFAULT_UNREST,
            duration: PENSION_DEFAULT_DURATION,
            label: "Unpaid pirate pensions".to_string(),
        });
        chronicle.send(AddChronicleEvent {
            text: format!(
                "PENSION DEFAULT: the colony could not pay its {} veteran pirate{} (shortfall {:.0}cr). The veterans are furious — unrest spreads.",
                ev.pirate_count,
                if ev.pirate_count == 1 { "" } else { "s" },
                ev.shortfall,
            ),
            importance: EventImportance::Major,
        });
    }
}

/// Counts living pops carrying the Pirate trait (for headless STATS).
#[must_use]
pub fn pirate_pop_count(world: &mut World) -> u32 {
    let mut query = world.query_filtered::<&Traits, With<Pop>>();
    #[allow(clippy::cast_possible_truncation)]
    let count = query
        .iter(world)
        .filter(|t| t.has(Trait::Pirate))
        .count();
    count as u32
}

/// Pension cost due each period for the current pirate population.
#[must_use]
pub fn pension_cost_per_period(world: &mut World) -> f32 {
    #[allow(clippy::cast_precision_loss)]
    let count = pirate_pop_count(world) as f32;
    count * PENSION_PER_PIRATE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pirate_crime() {
        let mut app = App::new();
        app.add_event::<CrimeCommittedEvent>();
        app.init_resource::<RelativitySimulationTime>();
        // Required so events clear each frame if added directly.
        // We will just read the cursor without clearing since we loop update.
        // Wait, Bevy clears events every frame if not explicitly updated in schedule via event buffer systems.
        // Actually, App::update() runs all schedules, including clearing events.
        // Let's read events directly instead of relying on `App::update()` loop OR don't run `App::update()`.
        // Better: Just manually run the system!

        let mut pirate_traits = Traits::default();
        pirate_traits.add(Trait::Pirate);

        // A pirate pop
        let pirate_pop = app.world_mut().spawn((Pop, pirate_traits)).id();

        // A normal pop
        let normal_pop = app.world_mut().spawn((Pop, Traits::default())).id();

        let mut pirate_crimes = 0;
        let mut normal_crimes = 0;

        // Run the system manually many times
        for _ in 0..1000 {
            let mut system_state = bevy::ecs::system::IntoSystem::into_system(pirate_crime_system);
            system_state.initialize(app.world_mut());
            system_state.run((), app.world_mut());
            system_state.apply_deferred(app.world_mut());

            let events = app.world().resource::<Events<CrimeCommittedEvent>>();
            let mut cursor = events.get_cursor();
            for ev in cursor.read(events) {
                if ev.perpetrator == pirate_pop {
                    pirate_crimes += 1;
                } else if ev.perpetrator == normal_pop {
                    normal_crimes += 1;
                }
            }
            app.world_mut()
                .resource_mut::<Events<CrimeCommittedEvent>>()
                .clear();
        }

        // Ensure pirate committed a crime and normal pop did not
        assert!(
            pirate_crimes > 0,
            "Pirate pop should have committed at least one crime"
        );
        assert_eq!(
            normal_crimes, 0,
            "Normal pop should not have committed a crime"
        );
    }

    #[test]
    fn test_process_pirate_amnesty() {
        let mut app = App::new();
        app.add_event::<PirateAmnestyEvent>();

        app.world_mut().send_event(PirateAmnestyEvent {
            fleet: Entity::PLACEHOLDER,
        });

        app.add_systems(Update, process_pirate_amnesty_system);
        app.update();

        // Verify that pirate pops were spawned as COMPLETE colonists
        // (GHOST-POP FIX 2026-10-04): Health, PopName, GridPosition —
        // not bare Pop+Traits+Wallet ghosts.
        let mut query = app.world_mut().query::<(
            &Pop,
            &Traits,
            &Wallet,
            &crate::layer1::pop::PopName,
            &crate::layer1::health::Health,
            &GridPosition,
        )>();
        let mut count = 0;
        for (_pop, traits, wallet, _name, _health, _pos) in query.iter(app.world()) {
            assert!(traits.has(Trait::Pirate));
            assert_eq!(wallet.credits, 1000.0);
            count += 1;
        }

        assert_eq!(count, 5); // Spawns 5 per fleet
    }

    // --- Spec 322 RED phase (adapted to real types) --------------------------

    #[test]
    fn test_amnestied_pirates_are_elite_veterans() {
        let mut app = App::new();
        app.add_event::<PirateAmnestyEvent>();
        app.world_mut().send_event(PirateAmnestyEvent {
            fleet: Entity::PLACEHOLDER,
        });
        app.add_systems(Update, process_pirate_amnesty_system);
        app.update();

        // Elite combat: every veteran keeps a cutlass dealing
        // VETERAN_CUTLASS_DAMAGE (normal migrants are unarmed: 0.0).
        let mut query = app
            .world_mut()
            .query::<(&Traits, &crate::layer1::items::Equipment, &Speed, &EdictCompliance, &PrivateStash)>();
        let mut count = 0;
        for (traits, equipment, speed, compliance, stash) in query.iter(app.world()) {
            assert!(traits.has(Trait::Pirate));
            let weapon_entity = equipment
                .weapon
                .expect("veteran pirate should be armed");
            let weapon = app.world().get::<Weapon>(weapon_entity).unwrap();
            assert!(
                (weapon.properties.damage - VETERAN_CUTLASS_DAMAGE).abs() < f32::EPSILON,
                "veteran cutlass damage should be {VETERAN_CUTLASS_DAMAGE}"
            );
            // Elite athletics: faster than a normal migrant (1.0).
            assert!(
                (speed.base - VETERAN_SPEED).abs() < f32::EPSILON,
                "veteran speed should be {VETERAN_SPEED}"
            );
            // Law-ignoring.
            assert!(compliance.ignores_safety);
            // PrivateStash present and owned.
            assert!(stash.owner.is_some());
            count += 1;
        }
        assert_eq!(count, PIRATES_PER_FLEET);
    }

    #[test]
    fn test_theft_probability_is_morale_and_hunger_driven() {
        let content = pirate_theft_probability(1.0, 1.0);
        let miserable = pirate_theft_probability(0.0, 1.0);
        let starving = pirate_theft_probability(1.0, 0.0);
        let worst = pirate_theft_probability(0.0, 0.0);
        // Never steals every tick: max probability stays well below 1.
        assert!(worst < 0.05, "theft must be probabilistic, got {worst}");
        // Base probability is positive but tiny for content, fed pirates.
        assert!(content > 0.0 && content < 0.01);
        // Misery and hunger both raise the odds.
        assert!(miserable > content, "low morale should raise theft odds");
        assert!(starving > content, "hunger should raise theft odds");
        assert!(worst > miserable && worst > starving);
    }

    #[test]
    fn test_pirates_steal_food_into_stash_probabilistically() {
        let mut app = App::new();
        app.add_event::<CrimeCommittedEvent>();
        app.insert_resource(ColonyResources {
            food: 100.0,
            ..Default::default()
        });

        let mut pirate_traits = Traits::default();
        pirate_traits.add(Trait::Pirate);
        let pirate = app
            .world_mut()
            .spawn((
                Pop,
                pirate_traits,
                Morale {
                    value: 0.0,
                    modifiers: Vec::new(),
                },
                Needs {
                    hunger: 0.0,
                    ..Default::default()
                },
                PrivateStash::default(),
            ))
            .id();
        // A control pop with no pirate trait and no stash.
        let _normal = app.world_mut().spawn((Pop, Traits::default())).id();

        // Miserable + starving pirate: ~1%/tick. Over 2000 ticks theft is
        // a near-certainty (P(no theft) ~= 0.99^2000 ~= 2e-9).
        for _ in 0..2000 {
            let mut system_state =
                bevy::ecs::system::IntoSystem::into_system(pirate_theft_system);
            system_state.initialize(app.world_mut());
            system_state.run((), app.world_mut());
            system_state.apply_deferred(app.world_mut());
        }

        let resources = app.world().resource::<ColonyResources>();
        let stash = app.world().get::<PrivateStash>(pirate).unwrap();
        let stolen = stash.get(ResourceType::Food);
        assert!(stolen > 0.0, "pirate should have stolen food");
        assert!(
            (resources.food - (100.0 - stolen)).abs() < 0.001,
            "stolen food must come from the colony stockpile"
        );

        // The theft is reported to the Justice System as Theft.
        let events = app.world().resource::<Events<CrimeCommittedEvent>>();
        let mut cursor = events.get_cursor();
        let thefts = cursor
            .read(events)
            .filter(|ev| {
                ev.perpetrator == pirate && matches!(ev.crime_type, CrimeType::Theft)
            })
            .count();
        assert!(thefts > 0, "theft should be reported as a crime");
    }

    #[test]
    fn test_pension_upkeep_drains_credits() {
        let mut app = App::new();
        app.add_event::<PensionDefaultEvent>();
        app.insert_resource(ColonyResources {
            credits: 1000.0,
            ..Default::default()
        });
        app.insert_resource(SimulationTime {
            tick: PENSION_PERIOD_TICKS,
            speed: crate::shared::time::SimSpeed::Normal,
        });

        let mut pirate_traits = Traits::default();
        pirate_traits.add(Trait::Pirate);
        // Two pirates: upkeep = 2 * 50 = 100.
        let _ = app.world_mut().spawn((Pop, pirate_traits.clone())).id();
        let _ = app.world_mut().spawn((Pop, pirate_traits)).id();

        let mut system_state =
            bevy::ecs::system::IntoSystem::into_system(process_pension_upkeep_system);
        system_state.initialize(app.world_mut());
        system_state.run((), app.world_mut());
        system_state.apply_deferred(app.world_mut());

        let resources = app.world().resource::<ColonyResources>();
        assert!(
            (resources.credits - 900.0).abs() < f32::EPSILON,
            "pension upkeep should drain 100 credits, got {}",
            resources.credits
        );
        // No default fired.
        let events = app.world().resource::<Events<PensionDefaultEvent>>();
        assert_eq!(events.len(), 0);
    }

    #[test]
    fn test_pension_upkeep_quiet_off_period_and_without_pirates() {
        let mut app = App::new();
        app.add_event::<PensionDefaultEvent>();
        app.insert_resource(ColonyResources {
            credits: 1000.0,
            ..Default::default()
        });
        // Off-period tick: no drain.
        app.insert_resource(SimulationTime {
            tick: 500,
            speed: crate::shared::time::SimSpeed::Normal,
        });
        let mut pirate_traits = Traits::default();
        pirate_traits.add(Trait::Pirate);
        let _ = app.world_mut().spawn((Pop, pirate_traits)).id();

        let mut system_state =
            bevy::ecs::system::IntoSystem::into_system(process_pension_upkeep_system);
        system_state.initialize(app.world_mut());
        system_state.run((), app.world_mut());
        system_state.apply_deferred(app.world_mut());
        assert_eq!(app.world().resource::<ColonyResources>().credits, 1000.0);

        // Period tick but zero pirates: no drain, no default.
        app.world_mut()
            .resource_mut::<SimulationTime>()
            .tick = PENSION_PERIOD_TICKS;
        let pirate_entity = {
            let mut query =
                app.world_mut()
                    .query_filtered::<Entity, (With<Pop>, With<Traits>)>();
            query.iter(app.world()).next().unwrap()
        };
        app.world_mut().despawn(pirate_entity);
        let mut system_state =
            bevy::ecs::system::IntoSystem::into_system(process_pension_upkeep_system);
        system_state.initialize(app.world_mut());
        system_state.run((), app.world_mut());
        system_state.apply_deferred(app.world_mut());
        assert_eq!(app.world().resource::<ColonyResources>().credits, 1000.0);
        assert_eq!(
            app.world()
                .resource::<Events<PensionDefaultEvent>>()
                .len(),
            0
        );
    }

    #[test]
    fn test_pension_default_zeroes_treasury_and_fires_event() {
        let mut app = App::new();
        app.add_event::<PensionDefaultEvent>();
        app.insert_resource(ColonyResources {
            credits: 10.0,
            ..Default::default()
        });
        app.insert_resource(SimulationTime {
            tick: PENSION_PERIOD_TICKS,
            speed: crate::shared::time::SimSpeed::Normal,
        });

        let mut pirate_traits = Traits::default();
        pirate_traits.add(Trait::Pirate);
        // Two pirates need 100cr; treasury holds 10.
        let _ = app.world_mut().spawn((Pop, pirate_traits.clone())).id();
        let _ = app.world_mut().spawn((Pop, pirate_traits)).id();

        let mut system_state =
            bevy::ecs::system::IntoSystem::into_system(process_pension_upkeep_system);
        system_state.initialize(app.world_mut());
        system_state.run((), app.world_mut());
        system_state.apply_deferred(app.world_mut());

        assert_eq!(app.world().resource::<ColonyResources>().credits, 0.0);
        let events = app.world().resource::<Events<PensionDefaultEvent>>();
        let mut cursor = events.get_cursor();
        let evs: Vec<_> = cursor.read(events).collect();
        assert_eq!(evs.len(), 1, "a default event should fire");
        assert_eq!(evs[0].pirate_count, 2);
        assert!((evs[0].shortfall - 90.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_pension_default_tanks_morale_and_feeds_unrest() {
        let mut app = App::new();
        app.add_event::<PensionDefaultEvent>();
        app.add_event::<AddChronicleEvent>();
        app.insert_resource(Unrest::default());

        let mut pirate_traits = Traits::default();
        pirate_traits.add(Trait::Pirate);
        let pirate = app
            .world_mut()
            .spawn((
                Pop,
                pirate_traits,
                Morale {
                    value: 0.8,
                    modifiers: Vec::new(),
                },
            ))
            .id();
        let normal = app
            .world_mut()
            .spawn((
                Pop,
                Traits::default(),
                Morale {
                    value: 0.8,
                    modifiers: Vec::new(),
                },
            ))
            .id();
        app.world_mut()
            .resource_mut::<Events<PensionDefaultEvent>>()
            .send(PensionDefaultEvent {
                pirate_count: 1,
                shortfall: 40.0,
            });

        let mut system_state =
            bevy::ecs::system::IntoSystem::into_system(process_pension_default_system);
        system_state.initialize(app.world_mut());
        system_state.run((), app.world_mut());
        system_state.apply_deferred(app.world_mut());

        // Pirate morale tanked; normal pop untouched.
        let pirate_morale = app.world().get::<Morale>(pirate).unwrap();
        assert!(
            pirate_morale
                .modifiers
                .iter()
                .any(|m| m.label == "Pension Defaulted" && m.value < 0.0),
            "pirate should carry the default morale hit"
        );
        let normal_morale = app.world().get::<Morale>(normal).unwrap();
        assert!(normal_morale.modifiers.is_empty());

        // Real unrest mechanism fed.
        let unrest = app.world().resource::<Unrest>();
        assert!(
            unrest
                .modifiers
                .iter()
                .any(|m| m.label == "Unpaid pirate pensions" && m.value > 0.0),
            "default should push an unrest modifier"
        );

        // Major chronicle written.
        let events = app.world().resource::<Events<AddChronicleEvent>>();
        let mut cursor = events.get_cursor();
        assert!(
            cursor.read(events).any(|ev| matches!(
                ev.importance,
                EventImportance::Major
            ) && ev.text.contains("PENSION DEFAULT")),
            "a Major chronicle should record the default"
        );
    }

    // --- IP guard ----------------------------------------------------------

    #[test]
    fn ip_guard_no_banned_terms() {
        // All player-facing strings this module introduces.
        let hay = [
            "PENSION DEFAULT: the colony could not pay its veteran pirates. The veterans are furious — unrest spreads.",
            "Pension Defaulted",
            "Unpaid pirate pensions",
        ]
        .join(" ")
        .to_lowercase();
        for banned in [
            "jack sparrow",
            "blackbeard",
            "barbarossa",
            "calico jack",
            "davy jones",
            "pirates of the caribbean",
            "one piece",
            "black sails",
            "flying dutchman",
        ] {
            assert!(!hay.contains(banned), "IP leak: {banned}");
        }
    }
}
