//! Spec 270: The Organ Market.
//!
//! The darkest possible logistics chain: spare parts.
//!
//! A [`BiomassExtractor`] building lets the colony harvest organs from the
//! dead. Harvested organs stockpile as `ResourceType::Organs` (perishable —
//! they rot), can be sold on the galactic market for credits, or used by
//! medical staff to instantly cure a critically injured pop. Every harvest
//! spreads horror: colony-wide morale damage plus a named mood modifier the
//! morale system decays over time, and a chronicle entry the colony remembers.
//!
//! Living harvests are deliberately narrow: only [`Inmate`] pops (the
//! justice system's genuine detained state) can be harvested alive, and only
//! through an explicit [`HarvestLivingEvent`] — the game never auto-harvests
//! the living.
//!
//! # Design notes
//!
//! * Stress flows through the REAL morale system (`Morale` + `MoodModifier`
//!   "Harvest Horror"), not a mock tracker. Psychopaths are immune.
//! * Organs are a real [`ColonyResources`] resource, sold for credits into
//!   [`EmpireResources`] (the same payout pattern as spec 1373 recalls).
//! * Perishability: stock decays `ORGAN_DECAY_PER_TICK` every tick.

use bevy_ecs::prelude::*;

use crate::layer1::architecture::building::{Building, BuildingType};
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::layer1::cryo_dreams::CryoTrauma;
use crate::layer1::economy::inflation::EmpireResources;
use crate::layer1::economy::resources::{ColonyResources, ResourceType};
use crate::layer1::funeral::Corpse;
use crate::layer1::health::Health;
use crate::layer1::law::justice::Inmate;
use crate::layer1::psychology::traits::{Trait, Traits};
use crate::layer1::radioactive::RadiationSickness;
use crate::layer1::social::morale::{MoodModifier, Morale};

/// Credits paid per organ sold on the galactic market.
pub const ORGAN_PRICE_CREDITS: f32 = 120.0;
/// Morale damage per corpse harvest (Morale.value is 0.0..1.0).
pub const CORPSE_HARVEST_STRESS: f32 = 0.25;
/// Morale damage per living-inmate harvest.
pub const LIVING_HARVEST_STRESS: f32 = 0.60;
/// How long the "Harvest Horror" mood modifier lingers (ticks).
pub const HORROR_DURATION_TICKS: u32 = 600;
/// Fraction of the organ stock that rots per tick (perishability).
pub const ORGAN_DECAY_PER_TICK: f32 = 0.001;
/// Max corpses processed per tick (keeps the horror metered).
pub const HARVEST_PER_TICK: usize = 2;
/// Health fraction at or below which a pop counts as critical for transplant.
pub const CRITICAL_HEALTH_FRACTION: f32 = 0.35;

/// Marker component for a Biomass Extractor building (Spec 270).
///
/// Inserted by building configuration when a `BuildingType::BiomassExtractor`
/// is constructed. Harvest systems key off its presence.
#[derive(Component, Default, Debug, Clone, Copy)]
pub struct BiomassExtractor;

/// Tuning knobs (and deterministic test hook) for the organ market.
#[derive(Resource, Debug, Clone)]
pub struct OrganMarketConfig {
    /// Credits paid per organ sold.
    pub organ_price_credits: f32,
    /// Morale damage per corpse harvest.
    pub corpse_stress: f32,
    /// Morale damage per living-inmate harvest.
    pub living_stress: f32,
    /// Fraction of organ stock that rots per tick.
    pub decay_per_tick: f32,
    /// Max corpses harvested per tick.
    pub harvest_per_tick: usize,
}

impl Default for OrganMarketConfig {
    fn default() -> Self {
        Self {
            organ_price_credits: ORGAN_PRICE_CREDITS,
            corpse_stress: CORPSE_HARVEST_STRESS,
            living_stress: LIVING_HARVEST_STRESS,
            decay_per_tick: ORGAN_DECAY_PER_TICK,
            harvest_per_tick: HARVEST_PER_TICK,
        }
    }
}

/// An organ was harvested (corpse or living donor). Drives the horror pass.
#[derive(Event, Debug, Clone)]
pub struct OrganHarvestEvent {
    /// Display name of the donor (corpse name or pop name).
    pub donor: String,
    /// True if the donor was alive (an Inmate) when harvested.
    pub living: bool,
}

/// Explicit request to harvest a living, detained pop.
///
/// The target MUST carry [`Inmate`]; the system refuses anything else.
/// (There is deliberately no auto-harvest of the living.)
#[derive(Event, Debug, Clone)]
pub struct HarvestLivingEvent {
    /// The inmate to harvest.
    pub target: Entity,
}

/// Sell harvested organs for credits.
#[derive(Event, Debug, Clone)]
pub struct SellOrgansEvent {
    /// How many organs to sell.
    pub quantity: u32,
}

/// Use one stored organ to instantly cure a critically injured pop.
#[derive(Event, Debug, Clone)]
pub struct TransplantOrganEvent {
    /// The patient to cure.
    pub patient: Entity,
}

/// Inflicts colony-wide horror: morale damage + a named, decaying mood
/// modifier. Psychopaths feel nothing.
fn apply_colony_horror(
    morale_query: &mut Query<(&mut Morale, Option<&Traits>)>,
    stress: f32,
    label: &str,
    duration: u32,
) {
    for (mut morale, traits_opt) in morale_query.iter_mut() {
        let is_psychopath = traits_opt.is_some_and(|t| t.0.contains(&Trait::Psychopath));
        if is_psychopath {
            continue;
        }
        morale.value = (morale.value - stress).max(0.0);
        // Refresh rather than stack the horror label.
        morale
            .modifiers
            .retain(|m| m.label != label);
        morale.add_modifier(MoodModifier {
            label: label.to_string(),
            value: -stress,
            duration,
        });
    }
}

/// Harvests organs from unburied corpses while a Biomass Extractor stands.
///
/// Each processed corpse yields 1 organ into the real `ColonyResources`,
/// despawns the corpse (like burial does), fires [`OrganHarvestEvent`], and
/// writes a chronicle entry. Capped at `harvest_per_tick` corpses per tick
/// so mass-casualty events meter their horror.
pub fn process_corpse_harvest_system(
    mut commands: Commands,
    extractors: Query<(&Building, &BiomassExtractor)>,
    corpses: Query<(Entity, &Corpse)>,
    config: Res<OrganMarketConfig>,
    resources: Option<ResMut<ColonyResources>>,
    mut harvest_events: EventWriter<OrganHarvestEvent>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    let extractor_live = extractors
        .iter()
        .any(|(b, _)| b.building_type == BuildingType::BiomassExtractor);
    if !extractor_live {
        return;
    }
    let Some(mut stock) = resources else { return };
    let mut harvested = 0;
    for (entity, corpse) in corpses.iter() {
        if harvested >= config.harvest_per_tick {
            break;
        }
        stock.add_resource(&ResourceType::Organs, 1.0);
        commands.entity(entity).despawn();
        harvested += 1;
        harvest_events.send(OrganHarvestEvent {
            donor: corpse.name.clone(),
            living: false,
        });
        chronicle.send(AddChronicleEvent {
            text: format!(
                "The Biomass Extractor processed the remains of {}. An organ was banked.",
                corpse.name
            ),
            importance: EventImportance::Standard,
        });
    }
}

/// Applies the morale horror for each harvest event (separate system so the
/// horror pass sees ALL harvests of the tick, including living ones).
pub fn apply_harvest_horror_system(
    mut events: EventReader<OrganHarvestEvent>,
    config: Res<OrganMarketConfig>,
    mut morale_query: Query<(&mut Morale, Option<&Traits>)>,
) {
    for ev in events.read() {
        let (stress, label) = if ev.living {
            (
                config.living_stress,
                "Harvest Horror (living donor)".to_string(),
            )
        } else {
            (
                config.corpse_stress,
                "Harvest Horror (corpse)".to_string(),
            )
        };
        apply_colony_horror(
            &mut morale_query,
            stress,
            &label,
            HORROR_DURATION_TICKS,
        );
    }
}

/// Harvests a living, detained pop on explicit request.
///
/// Refuses targets without [`Inmate`] — the game never harvests free pops.
/// Yields 1 organ, massive colony horror, and a Major chronicle entry.
#[allow(clippy::too_many_arguments)]
pub fn process_living_harvest_system(
    mut commands: Commands,
    mut events: EventReader<HarvestLivingEvent>,
    inmates: Query<&Inmate>,
    pops: Query<Entity>,
    resources: Option<ResMut<ColonyResources>>,
    mut harvest_events: EventWriter<OrganHarvestEvent>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    let Some(mut stock) = resources else { return };
    for ev in events.read() {
        if inmates.get(ev.target).is_err() {
            chronicle.send(AddChronicleEvent {
                text: "The Biomass Extractor refused a living harvest: the target is not detained."
                    .to_string(),
                importance: EventImportance::Minor,
            });
            continue;
        }
        if pops.get(ev.target).is_err() {
            continue;
        }
        stock.add_resource(&ResourceType::Organs, 1.0);
        commands.entity(ev.target).despawn();
        harvest_events.send(OrganHarvestEvent {
            donor: format!("inmate {:?}", ev.target),
            living: true,
        });
        chronicle.send(AddChronicleEvent {
            text: "A detained prisoner was harvested ALIVE in the Biomass Extractor. The colony will remember this."
                .to_string(),
            importance: EventImportance::Major,
        });
    }
}

/// Sells harvested organs for credits (pays only for stock actually on hand).
pub fn process_organ_sale_system(
    mut events: EventReader<SellOrgansEvent>,
    config: Res<OrganMarketConfig>,
    mut stockpile: Option<ResMut<ColonyResources>>,
    mut empires: Query<&mut EmpireResources>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    for ev in events.read() {
        let Some(stock) = stockpile.as_mut() else {
            continue;
        };
        // Organs are discrete: round (spoilage leaves fractional stock).
        let available = stock.get_amount(ResourceType::Organs).round().max(0.0) as u32;
        let sold = available.min(ev.quantity);
        if sold == 0 {
            continue;
        }
        stock.consume(ResourceType::Organs, sold as f32);
        let payout = sold as f32 * config.organ_price_credits;
        for mut empire in empires.iter_mut() {
            empire.credits += payout;
        }
        chronicle.send(AddChronicleEvent {
            text: format!(
                "Sold {sold} harvested organs on the galactic market for {payout:.0} credits."
            ),
            importance: EventImportance::Minor,
        });
    }
}

/// Uses one stored organ to instantly cure a critically injured pop.
///
/// Requires: at least 1 organ in stock, and the patient is critical
/// (`Health.current <= CRITICAL_HEALTH_FRACTION * max`) or afflicted
/// ([`CryoTrauma`] / [`RadiationSickness`]). Restores full health and clears
/// those afflictions.
pub fn process_transplant_system(
    mut commands: Commands,
    mut events: EventReader<TransplantOrganEvent>,
    mut patients: Query<(Entity, &mut Health)>,
    traumas: Query<&CryoTrauma>,
    sicknesses: Query<&RadiationSickness>,
    mut stockpile: Option<ResMut<ColonyResources>>,
    mut chronicle: EventWriter<AddChronicleEvent>,
) {
    let Some(stock) = stockpile.as_mut() else {
        return;
    };
    for ev in events.read() {
        let Ok((_, mut health)) = patients.get_mut(ev.patient) else {
            continue;
        };
        let critical = health.current <= CRITICAL_HEALTH_FRACTION * health.max;
        let afflicted = traumas.get(ev.patient).is_ok() || sicknesses.get(ev.patient).is_ok();
        if !critical && !afflicted {
            continue;
        }
        // Organs are discrete: a fractional stock (post-spoilage) still
        // covers one transplant; consume clamps at zero.
        if stock.get_amount(ResourceType::Organs).round() < 1.0 {
            chronicle.send(AddChronicleEvent {
                text: "Transplant failed: no organs in stock.".to_string(),
                importance: EventImportance::Minor,
            });
            continue;
        }
        stock.consume(ResourceType::Organs, 1.0);
        health.current = health.max;
        if traumas.get(ev.patient).is_ok() {
            commands.entity(ev.patient).remove::<CryoTrauma>();
        }
        if sicknesses.get(ev.patient).is_ok() {
            commands.entity(ev.patient).remove::<RadiationSickness>();
        }
        chronicle.send(AddChronicleEvent {
            text: "A donor organ was transplanted — a critically injured colonist walks away whole."
                .to_string(),
            importance: EventImportance::Standard,
        });
    }
}

/// Organs are perishable: the banked stock rots a fraction every tick.
pub fn organ_spoilage_system(
    config: Res<OrganMarketConfig>,
    stockpile: Option<ResMut<ColonyResources>>,
) {
    let Some(mut stock) = stockpile else { return };
    let current = stock.get_amount(ResourceType::Organs);
    if current > 0.0 {
        stock.consume(ResourceType::Organs, current * config.decay_per_tick);
    }
}

/// Registers the organ market events, resources, and systems on an App (tests).
pub fn register_organ_market(app: &mut bevy_app::App) {
    use bevy_app::Update;
    app.init_resource::<OrganMarketConfig>();
    app.add_event::<OrganHarvestEvent>();
    app.add_event::<HarvestLivingEvent>();
    app.add_event::<SellOrgansEvent>();
    app.add_event::<TransplantOrganEvent>();
    app.add_event::<AddChronicleEvent>();
    app.add_systems(
        Update,
        (
            process_corpse_harvest_system,
            apply_harvest_horror_system,
            process_living_harvest_system,
            process_organ_sale_system,
            process_transplant_system,
            organ_spoilage_system,
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::entities::pop::Pop;
    use bevy_ecs::system::RunSystemOnce;

    fn test_world() -> World {
        let mut world = World::new();
        world.init_resource::<OrganMarketConfig>();
        world.init_resource::<ColonyResources>();
        world.init_resource::<Events<OrganHarvestEvent>>();
        world.init_resource::<Events<HarvestLivingEvent>>();
        world.init_resource::<Events<SellOrgansEvent>>();
        world.init_resource::<Events<TransplantOrganEvent>>();
        world.init_resource::<Events<AddChronicleEvent>>();
        world
    }

    fn spawn_extractor(world: &mut World) {
        world.spawn((
            Building {
                building_type: BuildingType::BiomassExtractor,
                ..Default::default()
            },
            BiomassExtractor,
        ));
    }

    fn spawn_pop(world: &mut World, morale: f32) -> Entity {
        world
            .spawn((
                Pop,
                Morale {
                    value: morale,
                    ..Default::default()
                },
            ))
            .id()
    }

    #[test]
    fn red_corpse_harvest_yields_organ() {
        let mut world = test_world();
        spawn_extractor(&mut world);
        let corpse = world
            .spawn((
                Corpse {
                    name: "Test Colonist".to_string(),
                    decay: 0.0,
                },
            ))
            .id();

        let _ = RunSystemOnce::run_system_once(&mut world, process_corpse_harvest_system);

        let stock = world.resource::<ColonyResources>();
        assert_eq!(stock.get_amount(ResourceType::Organs), 1.0);
        assert!(world.get_entity(corpse).is_err());
    }

    #[test]
    fn red_corpse_harvest_needs_extractor() {
        let mut world = test_world();
        // No extractor built.
        let corpse = world
            .spawn((Corpse {
                name: "Test Colonist".to_string(),
                decay: 0.0,
            },))
            .id();

        let _ = RunSystemOnce::run_system_once(&mut world, process_corpse_harvest_system);

        let stock = world.resource::<ColonyResources>();
        assert_eq!(stock.get_amount(ResourceType::Organs), 0.0);
        assert!(world.get_entity(corpse).is_ok());
    }

    #[test]
    fn red_corpse_harvest_stresses_colony() {
        let mut world = test_world();
        spawn_extractor(&mut world);
        world.spawn((Corpse {
            name: "Test Colonist".to_string(),
            decay: 0.0,
        },));
        let witness = spawn_pop(&mut world, 0.8);

        let _ = RunSystemOnce::run_system_once(&mut world, process_corpse_harvest_system);
        let _ = RunSystemOnce::run_system_once(&mut world, apply_harvest_horror_system);

        let morale = world.get::<Morale>(witness).unwrap();
        assert!(morale.value < 0.8, "morale should drop: {}", morale.value);
        assert!(morale
            .modifiers
            .iter()
            .any(|m| m.label == "Harvest Horror (corpse)"));
    }

    #[test]
    fn red_living_harvest_requires_inmate() {
        let mut world = test_world();
        spawn_extractor(&mut world);
        let free_pop = spawn_pop(&mut world, 0.8);

        world
            .resource_mut::<Events<HarvestLivingEvent>>()
            .send(HarvestLivingEvent { target: free_pop });

        let _ = RunSystemOnce::run_system_once(&mut world, process_living_harvest_system);

        let stock = world.resource::<ColonyResources>();
        assert_eq!(stock.get_amount(ResourceType::Organs), 0.0);
        assert!(world.get_entity(free_pop).is_ok());
    }

    #[test]
    fn red_living_harvest_yields_organ_and_massive_stress() {
        let mut world = test_world();
        spawn_extractor(&mut world);
        let inmate = world.spawn((Pop, Inmate { sentence_ticks: 100 })).id();
        let witness = spawn_pop(&mut world, 0.8);

        world
            .resource_mut::<Events<HarvestLivingEvent>>()
            .send(HarvestLivingEvent { target: inmate });

        let _ = RunSystemOnce::run_system_once(&mut world, process_living_harvest_system);
        let _ = RunSystemOnce::run_system_once(&mut world, apply_harvest_horror_system);

        let stock = world.resource::<ColonyResources>();
        assert_eq!(stock.get_amount(ResourceType::Organs), 1.0);
        assert!(world.get_entity(inmate).is_err());
        let morale = world.get::<Morale>(witness).unwrap();
        assert!(
            morale.value <= 0.8 - 0.5,
            "living harvest should hit hard: {}",
            morale.value
        );
        assert!(morale
            .modifiers
            .iter()
            .any(|m| m.label == "Harvest Horror (living donor)"));
    }

    #[test]
    fn red_organs_sell_for_credits() {
        let mut world = test_world();
        world.spawn(EmpireResources {
            credits: 0.0,
            alloys: 0,
        });
        world
            .resource_mut::<ColonyResources>()
            .add_resource(&ResourceType::Organs, 3.0);
        world
            .resource_mut::<Events<SellOrgansEvent>>()
            .send(SellOrgansEvent { quantity: 2 });

        let _ = RunSystemOnce::run_system_once(&mut world, process_organ_sale_system);

        let stock = world.resource::<ColonyResources>();
        assert_eq!(stock.get_amount(ResourceType::Organs), 1.0);
        let mut q = world.query::<&EmpireResources>();
        let empire = q.iter(&world).next().unwrap();
        assert_eq!(empire.credits, 2.0 * ORGAN_PRICE_CREDITS);
    }

    #[test]
    fn red_transplant_cures_critical_patient() {
        let mut world = test_world();
        let patient = world
            .spawn((
                Pop,
                Health {
                    current: 10.0,
                    max: 100.0,
                    has_rust_lung: false,
                },
            ))
            .id();
        world
            .resource_mut::<ColonyResources>()
            .add_resource(&ResourceType::Organs, 1.0);
        world
            .resource_mut::<Events<TransplantOrganEvent>>()
            .send(TransplantOrganEvent { patient });

        let _ = RunSystemOnce::run_system_once(&mut world, process_transplant_system);

        let health = world.get::<Health>(patient).unwrap();
        assert_eq!(health.current, 100.0);
        let stock = world.resource::<ColonyResources>();
        assert_eq!(stock.get_amount(ResourceType::Organs), 0.0);
    }

    #[test]
    fn red_transplant_refuses_healthy_patient() {
        let mut world = test_world();
        let patient = world
            .spawn((
                Pop,
                Health {
                    current: 95.0,
                    max: 100.0,
                    has_rust_lung: false,
                },
            ))
            .id();
        world
            .resource_mut::<ColonyResources>()
            .add_resource(&ResourceType::Organs, 1.0);
        world
            .resource_mut::<Events<TransplantOrganEvent>>()
            .send(TransplantOrganEvent { patient });

        let _ = RunSystemOnce::run_system_once(&mut world, process_transplant_system);

        let stock = world.resource::<ColonyResources>();
        assert_eq!(
            stock.get_amount(ResourceType::Organs),
            1.0,
            "healthy patient must not consume an organ"
        );
    }

    #[test]
    fn red_organs_perish_over_time() {
        let mut world = test_world();
        world
            .resource_mut::<ColonyResources>()
            .add_resource(&ResourceType::Organs, 10.0);

        let _ = RunSystemOnce::run_system_once(&mut world, organ_spoilage_system);

        let stock = world.resource::<ColonyResources>();
        let after = stock.get_amount(ResourceType::Organs);
        assert!(
            (after - (10.0 * (1.0 - ORGAN_DECAY_PER_TICK))).abs() < 1e-4,
            "organs should rot: {after}"
        );
    }

    #[test]
    fn red_psychopaths_feel_no_horror() {
        let mut world = test_world();
        spawn_extractor(&mut world);
        let mut traits = Traits::default();
        traits.0.insert(Trait::Psychopath);
        let psycho = world
            .spawn((
                Pop,
                traits,
                Morale {
                    value: 0.8,
                    ..Default::default()
                },
            ))
            .id();

        let _ = RunSystemOnce::run_system_once(&mut world, process_corpse_harvest_system);
        let _ = RunSystemOnce::run_system_once(&mut world, apply_harvest_horror_system);

        // No corpses spawned in this test, so send a harvest event directly.
        world
            .resource_mut::<Events<OrganHarvestEvent>>()
            .send(OrganHarvestEvent {
                donor: "nobody".to_string(),
                living: false,
            });
        let _ = RunSystemOnce::run_system_once(&mut world, apply_harvest_horror_system);

        let morale = world.get::<Morale>(psycho).unwrap();
        assert_eq!(morale.value, 0.8);
        assert!(morale.modifiers.is_empty());
    }

    #[test]
    fn red_harvest_writes_chronicle() {
        let mut world = test_world();
        spawn_extractor(&mut world);
        world.spawn((Corpse {
            name: "Test Colonist".to_string(),
            decay: 0.0,
        },));

        let _ = RunSystemOnce::run_system_once(&mut world, process_corpse_harvest_system);

        let reader = world.resource::<Events<AddChronicleEvent>>();
        let mut cursor = reader.get_cursor();
        let count = cursor.read(reader).count();
        assert_eq!(count, 1, "harvest should write a chronicle entry");
    }

    #[test]
    fn red_sale_works_on_fractional_spoiled_stock() {
        // Regression: spoilage leaves e.g. 0.995 organs; the sale must still
        // count it as one discrete organ (floor() turned it into zero).
        let mut world = test_world();
        world.spawn(EmpireResources {
            credits: 0.0,
            alloys: 0,
        });
        world
            .resource_mut::<ColonyResources>()
            .add_resource(&ResourceType::Organs, 0.995);
        world
            .resource_mut::<Events<SellOrgansEvent>>()
            .send(SellOrgansEvent { quantity: 5 });

        let _ = RunSystemOnce::run_system_once(&mut world, process_organ_sale_system);

        let stock = world.resource::<ColonyResources>();
        assert_eq!(stock.get_amount(ResourceType::Organs), 0.0);
        let mut q = world.query::<&EmpireResources>();
        let empire = q.iter(&world).next().unwrap();
        assert_eq!(empire.credits, ORGAN_PRICE_CREDITS);
    }

    #[test]
    fn red_transplant_works_on_fractional_spoiled_stock() {
        let mut world = test_world();
        let patient = world
            .spawn((
                Pop,
                Health {
                    current: 10.0,
                    max: 100.0,
                    has_rust_lung: false,
                },
            ))
            .id();
        world
            .resource_mut::<ColonyResources>()
            .add_resource(&ResourceType::Organs, 0.995);
        world
            .resource_mut::<Events<TransplantOrganEvent>>()
            .send(TransplantOrganEvent { patient });

        let _ = RunSystemOnce::run_system_once(&mut world, process_transplant_system);

        let health = world.get::<Health>(patient).unwrap();
        assert_eq!(health.current, 100.0);
    }

    #[test]
    fn ip_guard_no_banned_terms() {
        // Concept-only IP rule: no lifted names in public strings.
        let hay = [
            "The Biomass Extractor processed the remains of",
            "A detained prisoner was harvested ALIVE in the Biomass Extractor. The colony will remember this.",
            "The Biomass Extractor refused a living harvest: the target is not detained.",
            "Sold harvested organs on the galactic market for",
            "A donor organ was transplanted — a critically injured colonist walks away whole.",
            "Transplant failed: no organs in stock.",
            "Harvest Horror (corpse)",
            "Harvest Horror (living donor)",
        ]
        .join("\n")
        .to_lowercase();
        for banned in [
            "donor organ black market",
            "repo man",
            "repo! the genetic opera",
            "organleggers",
            "larry niven",
            "the jigsaw man",
            "never let me go",
            "kazuo ishiguro",
        ] {
            assert!(!hay.contains(banned), "IP leak: {banned}");
        }
    }
}
