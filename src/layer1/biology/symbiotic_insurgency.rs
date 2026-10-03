use crate::layer1::psychology::needs::Needs;
use crate::layer1::psychology::traits::{Trait, Traits};
use bevy_ecs::prelude::*;

#[derive(Component)]
pub struct MindSporeInfection {
    pub active: bool,
}

/// Marker placed on an airlock whose doors the symbiont faction has forced
/// open. Counts down each tick; when it expires the crew wrestles the
/// airlock shut again (see `reseal_sabotaged_airlocks_system`).
#[derive(Component)]
pub struct SabotagedAirlock {
    pub ticks_remaining: u32,
}

/// How long a sabotaged airlock stays forced open before the crew reseals it.
pub const SABOTAGED_AIRLOCK_RESEAL_TICKS: u32 = 150;

/// Per-tick chance for an infected pop's immune system to fight off the
/// mind-spore infection on its own. Keeps the symbiont faction ebbing and
/// flowing instead of ratcheting to 100% and staying there forever.
pub const SPORE_CLEARANCE_CHANCE_PER_TICK: f64 = 0.004;

/// Base structure damage dealt to each LifeSupport building by one
/// air-filtration sabotage event, plus a per-member escalation term.
///
/// Tuned so a 5-pop colony can keep up: at full infection (5 members) this
/// is 27 damage per 10 ticks (2.7/tick), which one pop spending ~a third of
/// its time on Repair (10 HP/tick) can offset. Teeth, not a death sentence.
pub const SABOTAGE_BASE_DAMAGE: f32 = 12.0;
pub const SABOTAGE_DAMAGE_PER_MEMBER: f32 = 3.0;
pub const SABOTAGE_MAX_DAMAGE: f32 = 30.0;

#[derive(Resource, Default)]
pub struct SymbiontFaction {
    pub members: usize,
    pub critical_mass: usize,
}

#[derive(Event)]
pub struct SabotageEvent {
    pub target: SabotageTarget,
}

pub enum SabotageTarget {
    AirFiltration,
    Airlocks,
}

#[derive(Resource)]
pub struct InfectionConfig {
    pub base_transmission_rate: f32,
    pub flora_density_modifier: f32,
}

impl Default for InfectionConfig {
    fn default() -> Self {
        Self {
            base_transmission_rate: 0.05,
            flora_density_modifier: 0.02,
        }
    }
}

pub fn transmit_mind_spore_infection_system(
    config: Res<InfectionConfig>,
    mut commands: Commands,
    uninfected_query: Query<
        (Entity, &crate::layer1::entities::pop::Pop),
        (
            Without<MindSporeInfection>,
            With<crate::layer1::health::Health>,
        ),
    >,
    mut faction: ResMut<SymbiontFaction>,
) {
    use rand::Rng;
    let mut rng = rand::thread_rng();

    let active_carriers = faction.members;
    let probability = config.base_transmission_rate
        + (active_carriers as f32 * 0.01)
        + config.flora_density_modifier;

    for (entity, _) in uninfected_query.iter() {
        if rng.gen::<f32>() < probability {
            commands
                .entity(entity)
                .insert(MindSporeInfection { active: true });
            faction.members += 1;
        }
    }
}

pub fn process_mind_spore_infection_system(
    mut query: Query<(&mut Needs, &mut Traits, &MindSporeInfection)>,
) {
    for (mut needs, mut traits, infection) in query.iter_mut() {
        if infection.active {
            needs.hunger = 1.0;
            needs.rest = 1.0;
            needs.leisure = 1.0;
            needs.hygiene = 1.0;
            if !traits.has(Trait::MindSporeInfected) {
                traits.add(Trait::MindSporeInfected);
            }
        }
    }
}

pub fn trigger_symbiont_sabotage_system(
    faction: Res<SymbiontFaction>,
    mut events: EventWriter<SabotageEvent>,
    mut timer: Local<u32>,
) {
    *timer += 1;
    if *timer < 10 {
        return;
    }
    *timer = 0;

    if faction.members >= faction.critical_mass {
        events.send(SabotageEvent {
            target: SabotageTarget::Airlocks,
        });
    } else if faction.members > 0 {
        events.send(SabotageEvent {
            target: SabotageTarget::AirFiltration,
        });
    }
}

/// Immune response: infected pops have a small per-tick chance to fight off
/// the mind-spore infection on their own, shrinking the symbiont faction.
/// Transmission still outpaces clearance in a crowded colony, so the crisis
/// keeps its teeth — but the faction can now ebb as well as flow.
pub fn fight_off_spore_infection_system(
    mut commands: Commands,
    mut faction: ResMut<SymbiontFaction>,
    mut query: Query<(
        Entity,
        &MindSporeInfection,
        &mut Traits,
        Option<&crate::layer1::pop::PopName>,
    ), (
        With<crate::layer1::entities::pop::Pop>,
        With<crate::layer1::health::Health>,
        Without<crate::layer1::biology::health::Dead>,
    )>,
    mut chronicle: EventWriter<crate::layer1::core::chronicle::AddChronicleEvent>,
) {
    use crate::layer1::core::chronicle::EventImportance;
    use rand::Rng;
    let mut rng = rand::thread_rng();

    for (entity, infection, mut traits, name) in query.iter_mut() {
        if !infection.active {
            continue;
        }
        if rng.gen_bool(SPORE_CLEARANCE_CHANCE_PER_TICK) {
            commands.entity(entity).remove::<MindSporeInfection>();
            traits.remove(Trait::MindSporeInfected);
            faction.members = faction.members.saturating_sub(1);
            let who = name
                .map(|n| n.0.clone())
                .unwrap_or_else(|| "A colonist".to_string());
            chronicle.send(crate::layer1::core::chronicle::AddChronicleEvent {
                text: format!(
                    "{} shakes off the spore-dream; the symbiont's hold weakens.",
                    who
                ),
                importance: EventImportance::Minor,
            });
        }
    }
}

/// When an infected pop dies, release its hold on the symbiont faction
/// count. Without this, dead pops inflate `members` forever (and, before
/// the `Without<Dead>` filter above, corpses kept "fighting off" the
/// infection, pinning the faction near zero).
pub fn release_dead_spore_hosts_system(
    mut commands: Commands,
    mut faction: ResMut<SymbiontFaction>,
    query: Query<
        Entity,
        (
            With<MindSporeInfection>,
            Added<crate::layer1::biology::health::Dead>,
        ),
    >,
) {
    for entity in query.iter() {
        commands.entity(entity).remove::<MindSporeInfection>();
        faction.members = faction.members.saturating_sub(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::entities::pop::Pop;
    use crate::layer1::psychology::needs::Needs;
    use crate::layer1::psychology::traits::{get_trait_work_speed_modifier, Trait, Traits};
    use bevy::prelude::*;

    #[test]
    fn test_mind_spore_infection_boosts_morale_and_work_speed() {
        let mut app = App::new();
        app.add_systems(Update, process_mind_spore_infection_system);

        let pop = app
            .world_mut()
            .spawn((
                Pop,
                Needs {
                    hunger: 0.0,
                    rest: 0.0,
                    leisure: 0.0,
                    hygiene: 0.0,
                }, // Morale will be 0.0
                Traits::default(),
                MindSporeInfection { active: true },
            ))
            .id();

        app.update();

        let needs = app.world().get::<Needs>(pop).unwrap();
        let traits = app.world().get::<Traits>(pop).unwrap();

        assert!(
            (needs.morale() - 1.0).abs() < f32::EPSILON,
            "Infected pops must have boosted morale (Needs set to 1.0)"
        );
        assert!(
            traits.has(Trait::MindSporeInfected),
            "Infected pops must have MindSporeInfected trait"
        );

        let work_speed = get_trait_work_speed_modifier(traits);
        assert!(
            (work_speed - 1.5).abs() < f32::EPSILON,
            "Infected pops must work faster (1.5x)"
        );
    }

    #[test]
    fn test_symbiont_faction_growth_triggers_sabotage() {
        let mut app = App::new();
        app.add_event::<SabotageEvent>();
        app.add_systems(Update, trigger_symbiont_sabotage_system);

        app.world_mut().insert_resource(SymbiontFaction {
            members: 50,
            critical_mass: 40,
        });

        for _ in 0..10 {
            app.update();
        }

        let sabotage_events = app.world().resource::<Events<SabotageEvent>>();
        let mut reader = sabotage_events.get_cursor();
        let events: Vec<_> = reader.read(sabotage_events).collect();
        assert!(
            !events.is_empty(),
            "Critical mass symbiont faction must trigger sabotage"
        );

        // At critical mass, target should be Airlocks
        let target_is_airlocks = events
            .iter()
            .any(|e| matches!(e.target, SabotageTarget::Airlocks));
        assert!(target_is_airlocks, "Sabotage target should be Airlocks");
    }

    #[test]
    fn test_transmit_mind_spore_infection_system() {
        use crate::layer1::health::Health;
        let mut app = App::new();
        app.insert_resource(InfectionConfig {
            base_transmission_rate: 1.0, // 100% transmission for test
            flora_density_modifier: 0.0,
        });
        app.insert_resource(SymbiontFaction {
            members: 0,
            critical_mass: 40,
        });
        app.add_systems(Update, transmit_mind_spore_infection_system);

        let pop = app.world_mut().spawn((Pop, Health::default())).id();

        app.update();

        assert!(
            app.world().get::<MindSporeInfection>(pop).is_some(),
            "Pop should be infected"
        );
        assert_eq!(
            app.world().resource::<SymbiontFaction>().members,
            1,
            "Faction members should increase"
        );
    }

    #[test]
    fn test_trigger_symbiont_sabotage_system_below_critical() {        let mut app = App::new();
        app.add_event::<SabotageEvent>();
        app.add_systems(Update, trigger_symbiont_sabotage_system);

        app.world_mut().insert_resource(SymbiontFaction {
            members: 10,
            critical_mass: 40,
        });

        for _ in 0..10 {
            app.update();
        }

        let sabotage_events = app.world().resource::<Events<SabotageEvent>>();
        let mut reader = sabotage_events.get_cursor();
        let events: Vec<_> = reader.read(sabotage_events).collect();
        assert!(
            !events.is_empty(),
            "Non-zero symbiont faction must trigger sabotage"
        );

        let target_is_air_filtration = events
            .iter()
            .any(|e| matches!(e.target, SabotageTarget::AirFiltration));
        assert!(
            target_is_air_filtration,
            "Sabotage target should be AirFiltration"
        );
    }

    #[test]
    fn test_fight_off_spore_infection_clears_eventually() {
        use crate::layer1::core::chronicle::AddChronicleEvent;
        use crate::layer1::health::Health;
        let mut app = App::new();
        app.add_event::<AddChronicleEvent>();
        app.insert_resource(SymbiontFaction {
            members: 1,
            critical_mass: 40,
        });
        app.add_systems(Update, fight_off_spore_infection_system);

        let pop = app
            .world_mut()
            .spawn((
                Pop,
                Health::default(),
                Needs {
                    hunger: 1.0,
                    rest: 1.0,
                    leisure: 1.0,
                    hygiene: 1.0,
                },
                Traits::default(),
                MindSporeInfection { active: true },
            ))
            .id();

        // Run enough ticks that clearance (p=0.004/tick) is near-certain.
        for _ in 0..5000 {
            app.update();
            if app.world().get::<MindSporeInfection>(pop).is_none() {
                break;
            }
        }

        assert!(
            app.world().get::<MindSporeInfection>(pop).is_none(),
            "Infection should clear within 5000 ticks at p=0.004"
        );
        assert_eq!(
            app.world().resource::<SymbiontFaction>().members,
            0,
            "Faction members should decrement on clearance"
        );
    }
}
