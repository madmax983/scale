use crate::layer1::chronicle::{Chronicle, EventImportance};
use crate::layer1::map::GridPosition;
use crate::layer1::needs::Needs;
use crate::layer1::social::{AffinityChange, Tavern};
use crate::layer1::stress::StressTracker;
use crate::layer1::traits::{Trait, Traits};
use bevy_ecs::prelude::*;
use rand::seq::SliceRandom;
use std::collections::HashMap;

/// Topic of a rumor.
#[derive(Clone, Debug, PartialEq)]
pub enum RumorTopic {
    /// A shortage of a specific resource (e.g., "Food").
    ResourceShortage(String),
    /// Gossip about a specific character (Target, Affinity Modifier).
    CharacterGossip(Entity, f32),
    /// News about an event.
    EventNews(String),
    /// A prophecy of doom.
    DoomProphecy,
    /// A baseless scare story: mutants are nesting in the deep tunnels.
    /// Fabricated by stressed pops (Spec 343); believers panic.
    FalseMutantAttack,
    /// Word of a job opening at a specific workplace (Spec 343).
    JobOpening(Entity),
}

/// A piece of information that spreads through the colony.
#[derive(Clone, Debug)]
pub struct Rumor {
    /// The subject of the rumor.
    pub topic: RumorTopic,
    /// The original source of the rumor.
    pub source: Entity,
    /// The simulation tick when the rumor was created.
    pub timestamp: u64,
    /// The strength of the rumor (0.0 to 1.0), which may decay.
    pub strength: f32,
}

/// Component storing the rumors known by an entity.
#[derive(Component, Default)]
pub struct Knowledge {
    /// List of known rumors.
    pub known_rumors: Vec<Rumor>,
}

impl Knowledge {
    /// Checks if a rumor with the given topic is already known.
    #[must_use]
    pub fn knows(&self, topic: &RumorTopic) -> bool {
        self.known_rumors.iter().any(|r| r.topic == *topic)
    }

    /// Adds a rumor if it is not already known.
    pub fn add_rumor(&mut self, rumor: Rumor) {
        if !self.knows(&rumor.topic) {
            self.known_rumors.push(rumor);
        }
    }
}

/// System to generate rumors based on pop conditions (e.g., low morale).
pub fn generate_rumor_system(
    mut query: Query<(Entity, &Needs, &mut Knowledge, Option<&StressTracker>)>,
    time: Res<crate::shared::time::SimulationTime>,
    mut chronicle: Option<ResMut<Chronicle>>,
    mut ledger: Option<ResMut<RumorWebLedger>>,
) {
    for (entity, needs, mut knowledge, stress) in &mut query {
        if needs.morale() < 0.2 {
            // Angry/Depressed pop invents a rumor
            let rumor = Rumor {
                topic: RumorTopic::DoomProphecy,
                source: entity,
                timestamp: time.tick,
                strength: 1.0,
            };
            knowledge.add_rumor(rumor);
        }

        // Spec 343: a pop buckling under stress fabricates a false rumor —
        // a scare story with no source but fear itself.
        let buckling =
            stress.is_some_and(|s| s.accumulated_stress >= FALSE_RUMOR_STRESS_THRESHOLD);
        if buckling && !knowledge.knows(&RumorTopic::FalseMutantAttack) {
            knowledge.add_rumor(Rumor {
                topic: RumorTopic::FalseMutantAttack,
                source: entity,
                timestamp: time.tick,
                strength: 1.0,
            });

            // Chronicle the colony's first fabrication, once.
            let first = ledger
                .as_ref()
                .is_some_and(|l| !l.false_rumor_chronicled);
            if first {
                if let Some(ledger) = ledger.as_mut() {
                    ledger.false_rumor_chronicled = true;
                }
                if let Some(chronicle) = chronicle.as_mut() {
                    chronicle.add_event(
                        time.tick,
                        "A baseless scare story races through the colony: something                          with too many legs is nesting in the deep tunnels. Nobody                          has seen it. Everybody believes it. [FALSE_RUMOR]"
                            .to_string(),
                        EventImportance::Major,
                    );
                }
            }
        }
    }
}

/// Helper function to share a rumor from a speaker to a listener.
pub fn share_rumor(world: &mut World, speaker: Entity, listener: Entity) {
    // First, get a random rumor from speaker.
    let rumor_to_share = world.get::<Knowledge>(speaker).and_then(|k| {
        let mut rng = rand::thread_rng();
        k.known_rumors.choose(&mut rng).cloned()
    });

    if let Some(rumor) = rumor_to_share {
        // Emitting gossip event
        world.send_event(crate::layer1::social::gossip_economy::GossipEvent {
            pop: speaker,
            rumor: rumor.topic.clone(),
        });
        // Check if listener already knows it
        let already_knows = world
            .get::<Knowledge>(listener)
            .is_some_and(|k| k.knows(&rumor.topic));

        if !already_knows {
            world.send_event(crate::layer1::social::gossip_economy::GossipEvent {
                pop: listener,
                rumor: rumor.topic.clone(),
            });

            // Add rumor to listener
            if let Some(mut listener_knowledge) = world.get_mut::<Knowledge>(listener) {
                listener_knowledge.add_rumor(rumor.clone());
            }

            // Trigger Reaction
            process_rumor_reaction(world, listener, &rumor);
        }
    }
}

/// Processes the reaction of a listener to a newly heard rumor.
pub fn process_rumor_reaction(world: &mut World, listener: Entity, rumor: &Rumor) {
    match &rumor.topic {
        RumorTopic::CharacterGossip(target, amount) => {
            world.send_event(AffinityChange {
                source: listener,
                target: *target,
                amount: *amount,
            });
        }
        RumorTopic::JobOpening(_) => {
            // Word of work travels without emotional weight (Spec 343).
        }
        RumorTopic::ResourceShortage(_)
        | RumorTopic::DoomProphecy
        | RumorTopic::FalseMutantAttack => {
            if let Some(mut needs) = world.get_mut::<Needs>(listener) {
                // Simple morale hit
                needs.leisure = (needs.leisure - 0.05).max(0.0);
            }
        }
        RumorTopic::EventNews(ref news) => {
            if news == "Past Glory" {
                world.send_event(crate::layer1::culture::nostalgia::RumorSpreadEvent {
                    source: rumor.source,
                    target: listener,
                });
            }
        }
    }
}

/// System to exchange rumors between socializing pops.
///
/// Iterates over pops in Taverns and exchanges rumors between them.
pub fn exchange_rumors_system(world: &mut World) {
    // ⚡ Bolt Optimization:
    // Pre-allocated the exact required capacity for the `pairs` vector to prevent continuous heap re-allocations during hot frame updates.
    // Collect pairs directly to avoid cloning `Vec<Entity>` for every tavern.
    // This reduces allocations per frame from N+1 to 1 flat vector of pairs.
    let mut query = world.query::<&Tavern>();

    let capacity: usize = query
        .iter(world)
        .map(|t| {
            let n = t.visitors.len();
            if n >= 2 {
                n * (n - 1)
            } else {
                0
            }
        })
        .sum();

    let mut pairs = Vec::with_capacity(capacity);
    for tavern in query.iter(world) {
        if tavern.visitors.len() >= 2 {
            for &speaker in &tavern.visitors {
                for &listener in &tavern.visitors {
                    if speaker != listener {
                        pairs.push((speaker, listener));
                    }
                }
            }
        }
    }

    for (speaker, listener) in pairs {
        share_rumor(world, speaker, listener);
    }
}

#[cfg(test)]
mod tests {
    use crate::layer1::needs::Needs;
    use crate::layer1::pop::Pop;
    use crate::layer1::rumor::{generate_rumor_system, Knowledge, Rumor, RumorTopic};
    use crate::layer1::social::{AffinityChange, Relationships};
    use bevy_ecs::prelude::*;

    #[test]
    fn test_knowledge_default() {
        let knowledge = Knowledge::default();
        assert!(knowledge.known_rumors.is_empty());
    }

    #[test]
    fn test_rumor_struct() {
        let rumor = Rumor {
            topic: RumorTopic::ResourceShortage("Food".to_string()),
            source: Entity::PLACEHOLDER,
            timestamp: 100,
            strength: 1.0,
        };
        // Just verify it constructs
        assert_eq!(rumor.strength, 1.0);
    }

    #[test]
    fn test_rumor_generation_negative_morale() {
        let mut world = World::new();
        // Pop with very low morale (0.1)
        let pop = world
            .spawn((
                Pop,
                Needs {
                    hunger: 0.1,
                    rest: 0.1,
                    leisure: 0.1,
                    hygiene: 0.1,
                },
                Knowledge::default(),
            ))
            .id();

        // Run generation system
        let mut schedule = Schedule::default();
        schedule.add_systems(generate_rumor_system);
        // We need simulation time for timestamp
        world.insert_resource(crate::shared::time::SimulationTime {
            tick: 100,
            ..Default::default()
        });

        schedule.run(&mut world);

        // Check if rumor was generated
        let knowledge = world.get::<Knowledge>(pop).unwrap();
        assert!(
            !knowledge.known_rumors.is_empty(),
            "Rumor should be generated for low morale"
        );
        // Verify it's likely a negative rumor (e.g. Doom/Shortage)
        let rumor = &knowledge.known_rumors[0];
        match &rumor.topic {
            RumorTopic::ResourceShortage(_) | RumorTopic::DoomProphecy => {} // Pass
            _ => panic!("Expected negative rumor from low morale"),
        }
    }

    #[test]
    fn test_rumor_exchange_shares_info() {
        let mut world = World::new();
        world.init_resource::<Events<AffinityChange>>();

        let rumor = Rumor {
            topic: RumorTopic::EventNews("Colony Founded".to_string()),
            source: Entity::PLACEHOLDER,
            timestamp: 1,
            strength: 1.0,
        };

        // Pop A knows the rumor
        let pop_a = world
            .spawn((
                Pop,
                Knowledge {
                    known_rumors: vec![rumor.clone()],
                },
            ))
            .id();

        // Pop B knows nothing
        let pop_b = world.spawn((Pop, Knowledge::default())).id();

        // Simulating the effect of the system:
        crate::layer1::rumor::share_rumor(&mut world, pop_a, pop_b);

        let knowledge_b = world.get::<Knowledge>(pop_b).unwrap();
        assert_eq!(knowledge_b.known_rumors.len(), 1, "Rumor should be shared");
        assert_eq!(knowledge_b.known_rumors[0].topic, rumor.topic);
    }

    #[test]
    fn test_rumor_gossip_affects_affinity() {
        let mut world = World::new();
        world.init_resource::<Events<AffinityChange>>();

        let target_pop = world.spawn(Pop).id();
        let listener_pop = world
            .spawn((Pop, Knowledge::default(), Relationships::default()))
            .id();
        let speaker_pop = world.spawn(Pop).id();

        let rumor = Rumor {
            topic: RumorTopic::CharacterGossip(target_pop, -10.0), // Nasty gossip
            source: speaker_pop,
            timestamp: 1,
            strength: 1.0,
        };

        // Speaker tells Listener about Target
        // We expect AffinityChange event

        // Use helper to trigger logic
        crate::layer1::rumor::process_rumor_reaction(&mut world, listener_pop, &rumor);

        // Run system that processes events (from 047)
        // But here we just want to verify the event was sent.
        let events = world.resource::<Events<AffinityChange>>();
        // Using old get_reader for now as strict fix wasn't mandatory for functionality,
        // but if clippy complains I'll fix it.
        // Warning was about deprecated, not error.
        #[allow(deprecated)]
        let mut reader = events.get_reader();
        let emitted: Vec<_> = reader.read(events).collect();

        assert_eq!(emitted.len(), 1, "Should emit one AffinityChange event");
        assert_eq!(emitted[0].target, target_pop);
        assert!(emitted[0].amount < 0.0); // Affinity dropped
    }

    #[test]
    fn test_rumor_shortage_affects_morale() {
        let mut world = World::new();
        let listener = world
            .spawn((
                Pop,
                Needs {
                    leisure: 0.5,
                    hygiene: 0.8,
                    ..Default::default()
                },
                Knowledge::default(),
            ))
            .id();

        let rumor = Rumor {
            topic: RumorTopic::ResourceShortage("Food".to_string()),
            source: Entity::PLACEHOLDER,
            timestamp: 1,
            strength: 1.0,
        };

        crate::layer1::rumor::process_rumor_reaction(&mut world, listener, &rumor);

        let needs = world.get::<Needs>(listener).unwrap();
        assert!(needs.leisure < 0.5, "Leisure should drop on bad news");
    }

    #[test]
    fn test_rumor_doom_affects_morale() {
        let mut world = World::new();
        let listener = world
            .spawn((
                Pop,
                Needs {
                    leisure: 0.5,
                    hygiene: 0.8,
                    ..Default::default()
                },
                Knowledge::default(),
            ))
            .id();

        let rumor = Rumor {
            topic: RumorTopic::DoomProphecy,
            source: Entity::PLACEHOLDER,
            timestamp: 1,
            strength: 1.0,
        };

        crate::layer1::rumor::process_rumor_reaction(&mut world, listener, &rumor);

        let needs = world.get::<Needs>(listener).unwrap();
        assert!(needs.leisure < 0.5, "Leisure should drop on doom prophecy");
    }
}

// ============================================================================
// Spec 343: The Rumor Web — adjacency spread, false rumors, TTL, traits.
//
// Spec 055 gave the colony tavern-based rumor exchange (`Knowledge`,
// `share_rumor`, `exchange_rumors_system`). Spec 343 makes information a
// *physical* phenomenon: pops exchange knowledge tokens with whoever is
// standing next to them, stressed pops fabricate false rumors that panic
// their believers, rumors go stale, and personality (Gossip / Skeptic)
// shapes the web. Neighbor lookup uses a spatial hash grid — never an
// O(N^2) all-pairs scan (spec acceptance criterion).
// ============================================================================

/// Ticks a rumor stays fresh before pops forget it (Spec 343 TTL).
pub const RUMOR_TTL_TICKS: u64 = 600;
/// Stress spike inflicted on a pop that believes a false rumor.
pub const FALSE_RUMOR_PANIC_STRESS: f32 = 25.0;
/// Accumulated stress at which a pop starts fabricating false rumors.
pub const FALSE_RUMOR_STRESS_THRESHOLD: f32 = 50.0;
/// Chebyshev radius for ordinary face-to-face rumor exchange.
pub const RUMOR_SPREAD_RADIUS: i32 = 1;
/// Chebyshev radius when a Gossip is party to the exchange.
pub const GOSSIP_SPREAD_RADIUS: i32 = 2;

/// Ledger of rumor-web activity for the headless dashboard (Spec 343).
#[derive(Resource, Default, Debug)]
pub struct RumorWebLedger {
    /// Total rumor adoptions via adjacency spread.
    pub total_exchanges: u64,
    /// False rumors purged by Skeptics who saw through them.
    pub false_rumors_debunked: u64,
    /// Panic spikes inflicted by believed false rumors.
    pub panics: u64,
    /// Whether the first fabricated false rumor has been chronicled.
    pub false_rumor_chronicled: bool,
}

/// Whether a rumor topic is a fabrication (Spec 343 false rumor).
#[must_use]
pub fn is_false_rumor(topic: &RumorTopic) -> bool {
    matches!(topic, RumorTopic::FalseMutantAttack)
}

/// System: adjacent pops exchange rumors via a spatial hash grid (Spec 343).
///
/// Pops swap every rumor the other lacks. Ordinary pops exchange face to
/// face ([`RUMOR_SPREAD_RADIUS`]); when a [`Trait::Gossip`] is party to the
/// exchange it carries [`GOSSIP_SPREAD_RADIUS`]. Skeptics refuse false
/// rumors at the door. Transfers are computed from a per-tick snapshot, so
/// a rumor learned this tick only travels onward next tick — information
/// moves at walking speed, like it should.
pub fn spread_rumors_system(
    mut pops: Query<(Entity, &GridPosition, &mut Knowledge, Option<&Traits>)>,
    mut ledger: Option<ResMut<RumorWebLedger>>,
) {
    // Snapshot phase: positions, traits, and known rumors.
    struct PopSnapshot {
        entity: Entity,
        x: i32,
        y: i32,
        rumors: Vec<Rumor>,
        gossip: bool,
        skeptic: bool,
    }
    let mut snapshots: Vec<PopSnapshot> = Vec::new();
    for (entity, pos, knowledge, traits) in pops.iter() {
        snapshots.push(PopSnapshot {
            entity,
            x: pos.x,
            y: pos.y,
            rumors: knowledge.known_rumors.clone(),
            gossip: traits.is_some_and(|t| t.has(Trait::Gossip)),
            skeptic: traits.is_some_and(|t| t.has(Trait::Skeptic)),
        });
    }

    // Spatial hash grid: cell -> snapshot indices. Neighbor lookup only
    // ever scans the widest (gossip) neighborhood — never all pairs.
    let mut grid: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for (idx, snap) in snapshots.iter().enumerate() {
        grid.entry((snap.x, snap.y)).or_default().push(idx);
    }

    // Transfer phase: (listener, rumor) pairs to apply.
    let mut transfers: Vec<(Entity, Rumor)> = Vec::new();
    for (i, teller) in snapshots.iter().enumerate() {
        for dx in -GOSSIP_SPREAD_RADIUS..=GOSSIP_SPREAD_RADIUS {
            for dy in -GOSSIP_SPREAD_RADIUS..=GOSSIP_SPREAD_RADIUS {
                let Some(cell) = grid.get(&(teller.x + dx, teller.y + dy)) else {
                    continue;
                };
                for &j in cell {
                    if i == j {
                        continue;
                    }
                    let listener = &snapshots[j];
                    let chebyshev = (teller.x - listener.x)
                        .abs()
                        .max((teller.y - listener.y).abs());
                    let radius = if teller.gossip || listener.gossip {
                        GOSSIP_SPREAD_RADIUS
                    } else {
                        RUMOR_SPREAD_RADIUS
                    };
                    if chebyshev > radius {
                        continue;
                    }
                    for rumor in &teller.rumors {
                        // Skeptics refuse false rumors at the door.
                        if listener.skeptic && is_false_rumor(&rumor.topic) {
                            continue;
                        }
                        if !listener.rumors.iter().any(|r| r.topic == rumor.topic) {
                            transfers.push((listener.entity, rumor.clone()));
                        }
                    }
                }
            }
        }
    }

    // Apply phase: `add_rumor` dedups, so only count real adoptions.
    let mut exchanges: u64 = 0;
    for (listener, rumor) in transfers {
        if let Ok((_, _, mut knowledge, _)) = pops.get_mut(listener) {
            let before = knowledge.known_rumors.len();
            knowledge.add_rumor(rumor);
            if knowledge.known_rumors.len() > before {
                exchanges += 1;
            }
        }
    }
    if let Some(ledger) = ledger.as_mut() {
        ledger.total_exchanges += exchanges;
    }
}

/// System: pops who believe a false rumor panic (Spec 343).
///
/// Fires on `Changed<Knowledge>`: hearing (or re-hearing talk of) the
/// scare story spikes [`StressTracker::accumulated_stress`], feeding the
/// ordinary breakdown machinery. Pops with no stress tracker yet get one
/// — panic is how it starts.
pub fn react_to_rumors_system(
    mut commands: Commands,
    mut query: Query<(Entity, &Knowledge, Option<&mut StressTracker>), Changed<Knowledge>>,
    mut ledger: Option<ResMut<RumorWebLedger>>,
) {
    let mut panics: u64 = 0;
    for (entity, knowledge, tracker) in &mut query {
        if !knowledge.knows(&RumorTopic::FalseMutantAttack) {
            continue;
        }
        match tracker {
            Some(mut tracker) => {
                tracker.accumulated_stress += FALSE_RUMOR_PANIC_STRESS;
            }
            None => {
                commands.entity(entity).insert(StressTracker {
                    accumulated_stress: FALSE_RUMOR_PANIC_STRESS,
                });
            }
        }
        panics += 1;
    }
    if let Some(ledger) = ledger.as_mut() {
        ledger.panics += panics;
    }
}

/// System: rumors older than the TTL are forgotten; Skeptics purge false
/// rumors on sight (Spec 343).
///
/// The expiry check reads before it writes: pops with nothing stale keep
/// their `Knowledge` untouched, so the component is not marked `Changed`
/// every tick (which would re-trigger the panic reaction endlessly).
pub fn decay_rumors_system(
    mut pops: Query<(&mut Knowledge, Option<&Traits>)>,
    time: Option<Res<crate::shared::time::SimulationTime>>,
    mut ledger: Option<ResMut<RumorWebLedger>>,
) {
    let tick = time.map_or(0, |t| t.tick);
    let mut debunked: u64 = 0;
    for (mut knowledge, traits) in &mut pops {
        let skeptic = traits.is_some_and(|t| t.has(Trait::Skeptic));
        let has_expired = knowledge
            .known_rumors
            .iter()
            .any(|r| tick.saturating_sub(r.timestamp) > RUMOR_TTL_TICKS);
        let has_false = skeptic && knowledge.knows(&RumorTopic::FalseMutantAttack);
        if !has_expired && !has_false {
            continue;
        }
        if has_false {
            debunked += 1;
        }
        knowledge.known_rumors.retain(|r| {
            if skeptic && is_false_rumor(&r.topic) {
                return false;
            }
            tick.saturating_sub(r.timestamp) <= RUMOR_TTL_TICKS
        });
    }
    if let Some(ledger) = ledger.as_mut() {
        ledger.false_rumors_debunked += debunked;
    }
}

/// Total rumor instances known across all pops (headless STATS, Spec 343).
#[must_use]
pub fn known_rumor_count(world: &mut World) -> usize {
    let mut query = world.query::<&Knowledge>();
    query.iter(world).map(|k| k.known_rumors.len()).sum()
}

/// Pops currently believing a false rumor (headless STATS, Spec 343).
#[must_use]
pub fn false_rumor_believer_count(world: &mut World) -> usize {
    let mut query = world.query::<&Knowledge>();
    query
        .iter(world)
        .filter(|k| k.knows(&RumorTopic::FalseMutantAttack))
        .count()
}

#[cfg(test)]
mod spec_343_tests {
    use super::*;
    use crate::layer1::map::GridPosition;
    use crate::layer1::pop::Pop;
    use crate::layer1::stress::StressTracker;
    use crate::layer1::traits::{Trait, Traits};

    fn rumor(topic: RumorTopic) -> Rumor {
        Rumor {
            topic,
            source: Entity::PLACEHOLDER,
            timestamp: 0,
            strength: 1.0,
        }
    }

    fn food_rumor() -> Rumor {
        rumor(RumorTopic::ResourceShortage("Food".to_string()))
    }

    fn false_rumor() -> Rumor {
        rumor(RumorTopic::FalseMutantAttack)
    }

    fn run<Marker>(world: &mut World, system: impl IntoSystemConfigs<Marker>) {
        let mut schedule = Schedule::default();
        schedule.add_systems(system);
        schedule.run(world);
    }

    // Spec 343 RED test 1 (spec section 3, adapted to real types:
    // RumorTracker -> Knowledge, Transform -> GridPosition).
    #[test]
    fn test_adjacent_pops_exchange_rumors() {
        let mut world = World::new();

        let pop1 = world
            .spawn((
                Pop,
                GridPosition { x: 0, y: 0 },
                Knowledge {
                    known_rumors: vec![food_rumor()],
                },
            ))
            .id();
        let pop2 = world
            .spawn((Pop, GridPosition { x: 1, y: 0 }, Knowledge::default()))
            .id();

        run(&mut world, spread_rumors_system);

        let tracker2 = world.get::<Knowledge>(pop2).unwrap();
        assert!(
            tracker2.knows(&RumorTopic::ResourceShortage("Food".to_string())),
            "adjacent pop should learn the food-shortage rumor"
        );
        let _ = pop1;
    }

    #[test]
    fn test_exchange_is_bidirectional_and_carries_job_openings() {
        let mut world = World::new();

        let pop1 = world
            .spawn((
                Pop,
                GridPosition { x: 5, y: 5 },
                Knowledge {
                    known_rumors: vec![food_rumor()],
                },
            ))
            .id();
        let pop2 = world
            .spawn((
                Pop,
                GridPosition { x: 5, y: 6 },
                Knowledge {
                    known_rumors: vec![rumor(RumorTopic::JobOpening(pop1))],
                },
            ))
            .id();

        run(&mut world, spread_rumors_system);

        assert!(world
            .get::<Knowledge>(pop1)
            .unwrap()
            .knows(&RumorTopic::JobOpening(pop1)));
        assert!(world
            .get::<Knowledge>(pop2)
            .unwrap()
            .knows(&RumorTopic::ResourceShortage("Food".to_string())));
    }

    #[test]
    fn test_distant_pops_do_not_exchange() {
        let mut world = World::new();

        world.spawn((
            Pop,
            GridPosition { x: 0, y: 0 },
            Knowledge {
                known_rumors: vec![food_rumor()],
            },
        ));
        let pop2 = world
            .spawn((Pop, GridPosition { x: 10, y: 0 }, Knowledge::default()))
            .id();

        run(&mut world, spread_rumors_system);

        assert!(world.get::<Knowledge>(pop2).unwrap().known_rumors.is_empty());
    }

    // Spec 343 RED test 2 (spec section 3, adapted: Needs.stress ->
    // StressTracker.accumulated_stress, the real stress architecture).
    #[test]
    fn test_false_rumor_triggers_panic() {
        let mut world = World::new();

        let pop = world
            .spawn((
                Pop,
                Knowledge {
                    known_rumors: vec![false_rumor()],
                },
                StressTracker::default(),
            ))
            .id();

        run(&mut world, react_to_rumors_system);

        let stress = world.get::<StressTracker>(pop).unwrap();
        assert!(
            stress.accumulated_stress > 0.0,
            "believing a false rumor should spike stress, got {}",
            stress.accumulated_stress
        );
    }

    #[test]
    fn test_true_rumor_does_not_panic() {
        let mut world = World::new();

        let pop = world
            .spawn((
                Pop,
                Knowledge {
                    known_rumors: vec![food_rumor()],
                },
                StressTracker::default(),
            ))
            .id();

        run(&mut world, react_to_rumors_system);

        let stress = world.get::<StressTracker>(pop).unwrap();
        assert_eq!(stress.accumulated_stress, 0.0);
    }

    #[test]
    fn test_rumor_ttl_expires_old_rumors() {
        let mut world = World::new();
        world.insert_resource(crate::shared::time::SimulationTime {
            tick: RUMOR_TTL_TICKS + 1,
            ..Default::default()
        });

        let stale = world
            .spawn((
                Pop,
                Knowledge {
                    known_rumors: vec![food_rumor()],
                },
            ))
            .id();
        let fresh = world
            .spawn((
                Pop,
                Knowledge {
                    known_rumors: vec![Rumor {
                        timestamp: RUMOR_TTL_TICKS + 1,
                        ..food_rumor()
                    }],
                },
            ))
            .id();

        run(&mut world, decay_rumors_system);

        assert!(
            world.get::<Knowledge>(stale).unwrap().known_rumors.is_empty(),
            "rumor older than TTL should be forgotten"
        );
        assert_eq!(world.get::<Knowledge>(fresh).unwrap().known_rumors.len(), 1);
    }

    #[test]
    fn test_gossip_trait_spreads_further() {
        // Gossip carrier reaches a listener two tiles away.
        let mut world = World::new();
        let mut traits = Traits::default();
        traits.add(Trait::Gossip);
        world.spawn((
            Pop,
            GridPosition { x: 0, y: 0 },
            Knowledge {
                known_rumors: vec![food_rumor()],
            },
            traits,
        ));
        let listener = world
            .spawn((Pop, GridPosition { x: 2, y: 0 }, Knowledge::default()))
            .id();

        run(&mut world, spread_rumors_system);

        assert!(
            world
                .get::<Knowledge>(listener)
                .unwrap()
                .knows(&RumorTopic::ResourceShortage("Food".to_string())),
            "gossip should carry a rumor two tiles"
        );

        // Control: a non-gossip cannot reach two tiles.
        let mut world = World::new();
        world.spawn((
            Pop,
            GridPosition { x: 0, y: 0 },
            Knowledge {
                known_rumors: vec![food_rumor()],
            },
        ));
        let listener = world
            .spawn((Pop, GridPosition { x: 2, y: 0 }, Knowledge::default()))
            .id();

        run(&mut world, spread_rumors_system);

        assert!(world.get::<Knowledge>(listener).unwrap().known_rumors.is_empty());
    }

    #[test]
    fn test_skeptic_rejects_false_rumor_but_learns_true_ones() {
        let mut world = World::new();

        world.spawn((
            Pop,
            GridPosition { x: 0, y: 0 },
            Knowledge {
                known_rumors: vec![false_rumor(), food_rumor()],
            },
        ));
        let mut traits = Traits::default();
        traits.add(Trait::Skeptic);
        let skeptic = world
            .spawn((
                Pop,
                GridPosition { x: 1, y: 0 },
                Knowledge::default(),
                traits,
            ))
            .id();

        run(&mut world, spread_rumors_system);

        let knowledge = world.get::<Knowledge>(skeptic).unwrap();
        assert!(
            !knowledge.knows(&RumorTopic::FalseMutantAttack),
            "skeptic should refuse the false rumor"
        );
        assert!(
            knowledge.knows(&RumorTopic::ResourceShortage("Food".to_string())),
            "skeptic still learns true rumors"
        );
    }

    #[test]
    fn test_skeptic_purges_known_false_rumor() {
        let mut world = World::new();
        world.insert_resource(crate::shared::time::SimulationTime {
            tick: 10,
            ..Default::default()
        });

        let mut traits = Traits::default();
        traits.add(Trait::Skeptic);
        let skeptic = world
            .spawn((
                Pop,
                Knowledge {
                    known_rumors: vec![false_rumor(), food_rumor()],
                },
                traits,
            ))
            .id();

        run(&mut world, decay_rumors_system);

        let knowledge = world.get::<Knowledge>(skeptic).unwrap();
        assert!(!knowledge.knows(&RumorTopic::FalseMutantAttack));
        assert!(knowledge.knows(&RumorTopic::ResourceShortage("Food".to_string())));
    }

    #[test]
    fn test_stressed_pop_fabricates_false_rumor() {
        let mut world = World::new();
        world.insert_resource(crate::shared::time::SimulationTime {
            tick: 5,
            ..Default::default()
        });

        let pop = world
            .spawn((
                Pop,
                Needs::default(),
                Knowledge::default(),
                StressTracker {
                    accumulated_stress: FALSE_RUMOR_STRESS_THRESHOLD + 25.0,
                },
            ))
            .id();

        run(&mut world, generate_rumor_system);

        assert!(
            world
                .get::<Knowledge>(pop)
                .unwrap()
                .knows(&RumorTopic::FalseMutantAttack),
            "a pop past the stress threshold should fabricate a false rumor"
        );
    }

    #[test]
    fn test_calm_pop_does_not_fabricate() {
        let mut world = World::new();
        world.insert_resource(crate::shared::time::SimulationTime {
            tick: 5,
            ..Default::default()
        });

        let pop = world
            .spawn((
                Pop,
                Needs::default(),
                Knowledge::default(),
                StressTracker::default(),
            ))
            .id();

        run(&mut world, generate_rumor_system);

        assert!(!world
            .get::<Knowledge>(pop)
            .unwrap()
            .knows(&RumorTopic::FalseMutantAttack));
    }
}
