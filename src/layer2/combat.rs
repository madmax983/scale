use crate::layer1::resources::ResourceType;
use crate::layer2::events::ShipDestroyedEvent;
use crate::layer2::fleet::{FleetComposition, FleetFaction, InOrbit};
use crate::layer2::ship::ShipType;
use crate::layer2::system::OrbitalBody;
use crate::shared::log::{Message, MessageLog};
use bevy_ecs::prelude::*;
use ratatui::style::Color;

/// Result of a fleet combat engagement.
#[derive(Debug, Clone, PartialEq)]
pub struct CombatResult {
    /// The faction that won the engagement.
    pub winner: FleetFaction,
    /// The composition of the winning fleet after combat.
    pub winner_survivors: FleetComposition,
    /// The composition of the losing fleet after combat (if any survived).
    pub loser_survivors: FleetComposition,
    /// List of ships destroyed during combat (from both sides).
    pub destroyed_ships: Vec<ShipType>,
    /// Loot generated from the engagement (e.g. from destroyed transport ships).
    pub loot: Option<Vec<(ResourceType, f32)>>,
}

/// Resolves a combat encounter between two fleets.
///
/// Determines the winner based on total attack power.
/// Applies damage to both sides based on the opponent's power.
/// Generates loot if valuable ships are destroyed.
#[must_use]
pub fn resolve_combat(
    att_faction: FleetFaction,
    attacker: &FleetComposition,
    def_faction: FleetFaction,
    defender: &FleetComposition,
) -> CombatResult {
    let att_power: f32 = attacker
        .ships
        .iter()
        .map(|s| s.ship_type.attack_power())
        .sum();
    let def_power: f32 = defender
        .ships
        .iter()
        .map(|s| s.ship_type.attack_power())
        .sum();

    // Attacker wins ties
    let attacker_wins = att_power >= def_power;

    let (winner_faction, winner_comp, loser_comp, winner_power, loser_power) = if attacker_wins {
        (att_faction, attacker, defender, att_power, def_power)
    } else {
        (def_faction, defender, attacker, def_power, att_power)
    };

    // Damage Calculation
    // Winner takes damage = Loser Power * 0.5 (Mitigation)
    // Loser takes damage = Winner Power * 1.5 (Overwhelming force)
    let winner_damage = loser_power * 0.5;
    let loser_damage = winner_power * 1.5;

    let mut winner_survivors = winner_comp.clone();
    let winner_destroyed = winner_survivors.take_damage(winner_damage);

    let mut loser_survivors = loser_comp.clone();
    let loser_destroyed = loser_survivors.take_damage(loser_damage);

    // Loot Generation
    // If any Transport ships were destroyed, generate loot.
    let mut loot = Vec::new();
    let mut scrap_amount = 0.0;

    for ship_type in &loser_destroyed {
        // Scavenge raw materials
        let cost = ship_type.construction_cost();
        for (_, amount) in cost {
            scrap_amount += amount * 0.1; // 10% recovery
        }

        // Bonus for Transports
        if *ship_type == ShipType::Transport {
            scrap_amount += 100.0; // Cargo loot
        }
    }

    if scrap_amount > 0.0 {
        loot.push((ResourceType::Scrap, scrap_amount));
    }

    // Collect all destroyed ships
    let mut destroyed_ships = winner_destroyed;
    destroyed_ships.extend(loser_destroyed);

    CombatResult {
        winner: winner_faction,
        winner_survivors,
        loser_survivors,
        destroyed_ships,
        loot: if loot.is_empty() { None } else { Some(loot) },
    }
}

/// Player-facing label for a combat location (item 162): the
/// orbital body's name when it has one, else the `body <index>`
/// label used by the launch ledger. Never an Entity Debug rendering.
#[must_use]
pub fn combat_location_label(body_name: Option<&str>, location: Entity) -> String {
    body_name.map_or_else(|| format!("body {}", location.index()), ToString::to_string)
}

/// System to resolve combat between fleets at the same orbital location.
///
/// ⚡ Bolt Optimization:
/// Previously, this function cloned the `FleetComposition` of every fleet in the query
/// into a flat `Vec` every time the system ran. By switching to a grouped `HashMap`
/// approach that borrows `&FleetComposition`, we eliminate O(N) heap allocations
/// per frame/tick, cloning only when actual combat resolution takes place.
pub fn fleet_combat_system(
    mut commands: Commands,
    query: Query<(
        Entity,
        &InOrbit,
        &FleetFaction,
        &FleetComposition,
        Option<&AvoidCombat>,
    )>,
    bodies: Query<&OrbitalBody>,
    mut event_writer: EventWriter<ShipDestroyedEvent>,
    mut message_log: Option<ResMut<MessageLog>>,
) {
    use bevy::utils::HashMap;

    // Group fleets by location
    let mut location_groups: HashMap<Entity, Vec<(Entity, FleetFaction, &FleetComposition, bool)>> =
        HashMap::new();

    for (entity, in_orbit, faction, comp, avoid) in query.iter() {
        location_groups.entry(in_orbit.parent).or_default().push((
            entity,
            *faction,
            comp,
            avoid.is_some(),
        ));
    }

    for (location, fleets) in location_groups {
        if fleets.len() <= 1 {
            continue;
        }

        // Find first hostile pair
        let mut combat_pair = None;
        for j in 0..fleets.len() {
            for k in j + 1..fleets.len() {
                if fleets[j].1 != fleets[k].1 {
                    combat_pair = Some((j, k));
                    break;
                }
            }
            if combat_pair.is_some() {
                break;
            }
        }

        if let Some((idx1, idx2)) = combat_pair {
            let (e1, f1, c1, a1) = fleets[idx1];
            let (e2, f2, c2, a2) = fleets[idx2];

            if a1 || a2 {
                continue; // One fleet avoiding combat
            }

            // Resolve Combat
            let result = resolve_combat(f1, c1, f2, c2);

            let (winner_entity, loser_entity) = if result.winner == f1 {
                (e1, e2)
            } else {
                (e2, e1)
            };

            // Emit destroyed events
            for ship in &result.destroyed_ships {
                event_writer.send(ShipDestroyedEvent {
                    planet: location,
                    ship_class: format!("{ship:?}"),
                });
            }

            // Update Winner
            if result.winner_survivors.ships.is_empty() {
                commands.entity(winner_entity).despawn();
            } else {
                commands
                    .entity(winner_entity)
                    .insert(result.winner_survivors);
            }

            // Update Loser
            let loser_survivor_count = result.loser_survivors.ships.len();

            if result.loser_survivors.ships.is_empty() {
                commands.entity(loser_entity).despawn();
                // Debris spawn placeholder
            } else {
                commands.entity(loser_entity).insert(result.loser_survivors);
            }

            if let Some(log) = message_log.as_mut() {
                // Item 162 audit: `location` is an Entity — its Debug
                // rendering (`34v1#4294967330`) is not player-facing
                // text. Name the orbital body instead, falling back to
                // the `body <index>` label used by the launch ledger.
                // `result.winner` is a FleetFaction: its Debug is the
                // plain variant name, which is fine.
                let location_label = combat_location_label(
                    bodies.get(location).ok().map(|body| body.name.as_str()),
                    location,
                );
                log.messages.push_back(Message {
                    text: format!(
                        "Combat at {location_label}! Winner: {:?}, Loser Survivors: {}",
                        result.winner, loser_survivor_count
                    ),
                    color: Color::Red,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::layer2::combat::{combat_location_label, fleet_combat_system, resolve_combat};
    use crate::layer2::events::ShipDestroyedEvent;
    use crate::layer2::fleet::{FleetComposition, FleetFaction, InOrbit};
    use crate::layer2::ship::{Ship, ShipType};
    use crate::layer2::system::OrbitalBody;
    use crate::shared::log::MessageLog;
    use bevy_ecs::prelude::*;

    // Helper to create a test fleet
    fn create_fleet(_faction: FleetFaction, ships: Vec<ShipType>) -> FleetComposition {
        let mut comp = FleetComposition::default();
        for t in ships {
            comp.add_ship(Ship::new(t));
        }
        comp
    }

    #[test]
    fn test_combat_resolution_stronger_wins() {
        // Fleet A: 10 Frigates (High Combat)
        let fleet_a = create_fleet(FleetFaction::Player, vec![ShipType::Frigate; 10]);

        // Fleet B: 1 Scout (Low Combat)
        let fleet_b = create_fleet(FleetFaction::Pirate, vec![ShipType::Scout; 1]);

        // Resolve
        let result = resolve_combat(
            FleetFaction::Player,
            &fleet_a,
            FleetFaction::Pirate,
            &fleet_b,
        );

        // A should win
        assert_eq!(result.winner, FleetFaction::Player);
        // B should be wiped out (empty list of survivors)
        assert!(
            result.loser_survivors.ships.is_empty(),
            "Loser should have 0 survivors"
        );
        // A should take minimal/no damage (just checking count for now)
        // 10 Frigates vs 1 Scout. Scout power = 2. Frigate power = 50. Total A=500, B=2.
        // Winner damage = 2 * 0.5 = 1.0. Frigate HP = 100. No ships lost.
        assert_eq!(
            result.winner_survivors.ships.len(),
            10,
            "Winner should keep ships"
        );
    }

    #[test]
    fn test_combat_casualties() {
        // Fleet A: 5 Frigates
        let fleet_a = create_fleet(FleetFaction::Player, vec![ShipType::Frigate; 5]);

        // Fleet B: 5 Frigates
        let fleet_b = create_fleet(FleetFaction::Pirate, vec![ShipType::Frigate; 5]);

        // Even fight, both sides should take losses
        let result = resolve_combat(
            FleetFaction::Player,
            &fleet_a,
            FleetFaction::Pirate,
            &fleet_b,
        );

        // Winner Power: 250. Loser Power: 250.
        // Winner Damage: 125. (1.25 Frigates lost) -> 4 survivors (1 damaged).
        // Loser Damage: 375. (3.75 Frigates lost) -> 2 survivors (1 damaged).

        let total_survivors =
            result.winner_survivors.ships.len() + result.loser_survivors.ships.len();
        assert!(
            total_survivors < 10,
            "Casualties should occur in even fight"
        );
    }

    #[test]
    fn test_combat_location_label_names_body_not_entity() {
        // Item 162 regression: the label is the body's name, and the
        // fallback is a plain index — never `34v1#4294967330`.
        let mut world = World::new();
        let planet = world.spawn_empty().id();
        assert_eq!(
            combat_location_label(Some("Test Prime"), planet),
            "Test Prime"
        );
        let fallback = combat_location_label(None, planet);
        assert_eq!(fallback, format!("body {}", planet.index()));
        assert!(!fallback.contains('#'), "label leaked Entity Debug: {fallback}");
    }

    #[test]
    fn test_fleet_combat_log_names_orbital_body_not_entity() {
        // Item 162 regression, through the real system + MessageLog.
        let mut world = World::new();
        world.init_resource::<Events<ShipDestroyedEvent>>();
        world.insert_resource(MessageLog::default());
        let planet = world
            .spawn(OrbitalBody {
                name: "Test Prime".to_string(),
                ..Default::default()
            })
            .id();
        world.spawn((
            FleetFaction::Player,
            InOrbit { parent: planet },
            create_fleet(FleetFaction::Player, vec![ShipType::Frigate; 10]),
        ));
        world.spawn((
            FleetFaction::Pirate,
            InOrbit { parent: planet },
            create_fleet(FleetFaction::Pirate, vec![ShipType::Scout; 1]),
        ));
        let mut schedule = Schedule::default();
        schedule.add_systems(fleet_combat_system);
        schedule.run(&mut world);
        let log = world.resource::<MessageLog>();
        let texts: Vec<&str> = log.messages.iter().map(|m| m.text.as_str()).collect();
        assert!(
            texts.iter().any(|t| t.contains("Combat at Test Prime!")),
            "combat log should name the body, got: {texts:?}"
        );
        for text in &texts {
            assert!(!text.contains('#'), "combat log leaked Entity Debug: {text}");
            assert!(
                !text.contains(&format!("{planet:?}")),
                "combat log leaked Entity Debug: {text}"
            );
        }
    }

    #[test]
    fn test_loot_generation() {
        let fleet_a = create_fleet(FleetFaction::Player, vec![ShipType::Frigate; 10]);
        let fleet_b = create_fleet(FleetFaction::Pirate, vec![ShipType::Transport; 1]); // Transport has high cargo

        let result = resolve_combat(
            FleetFaction::Player,
            &fleet_a,
            FleetFaction::Pirate,
            &fleet_b,
        );

        // If Transport destroyed, loot generated
        assert!(
            result.loot.is_some(),
            "Loot should be generated when transport is destroyed"
        );
    }

    #[test]
    fn test_destroyed_ships_tracking() {
        let fleet_a = create_fleet(FleetFaction::Player, vec![ShipType::Frigate; 10]);
        let fleet_b = create_fleet(FleetFaction::Pirate, vec![ShipType::Scout; 1]);

        let result = resolve_combat(
            FleetFaction::Player,
            &fleet_a,
            FleetFaction::Pirate,
            &fleet_b,
        );

        // B's scout should be in destroyed list
        assert!(
            !result.destroyed_ships.is_empty(),
            "Destroyed ships should be tracked"
        );
        assert!(
            result.destroyed_ships.contains(&ShipType::Scout),
            "Scout should be destroyed"
        );
    }
}

#[derive(Component, Debug, Clone, Copy)]
pub struct AvoidCombat;
