//! Adventurer origin selection and randomized spawn scheduling.
//!
//! Mark's call (2026-10-06): the player chooses ONE origin at game start
//! (TUI setup screen, or headless `--origin` / the `origin` console
//! command). Without a choice, only 2–3 origins appear per playthrough —
//! two at start, staggered so their site scans don't collide on the same
//! deterministic ring tiles, plus one that trickles in later with a
//! chronicle rumor. This replaces the old behavior where all 8 origins
//! spawned in every game on one deterministic ring scan, clustering the
//! sites and letting the earlier-origin `interact` blocks always win.
//!
//! The [`origin_spawn_director`] exclusive system replaces the eight
//! unconditional `spawn_*_once` schedule entries in `simulation.rs`. The
//! per-origin spawn functions keep their own once-guards, so the director
//! only ever schedules each origin once.

use bevy_ecs::prelude::*;
use rand::seq::SliceRandom;
use rand::Rng;

use super::bloomtouched::{ensure_bloomtouched_resources, spawn_bloom_scar_once, BloomZone};
use super::chronostalker::{
    ensure_chronostalker_resources, spawn_moment_wound_once, ChronostalkerState,
};
use super::corsair::{ensure_corsair_resources, spawn_raider_skiff_once, CorsairState};
use super::governor::{ensure_governor_resources, spawn_appointment_seal_once, GovernorState};
use super::improbable::{ensure_pilot_resources, spawn_shuttle_once, PilotState};
use super::lawbound::{ensure_lawbound_resources, spawn_lawbound_cradle_once, LawboundState};
use super::salvager::{ensure_salvager_resources, spawn_derelict_hulk_once, SalvagerState};
use super::sovereign::{ensure_sovereign_resources, spawn_dented_crown_once, SovereignState};
use crate::layer1::core::chronicle::{AddChronicleEvent, EventImportance};
use crate::shared::time::SimulationTime;

/// The eight adventurer origins, in stable registration order.
///
/// The order doubles as `interact`-dispatch priority (earliest first), so
/// it must stay stable across runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OriginKind {
    /// Exiled-monarch possess path (the dented crown).
    Sovereign,
    /// Space-pirate possess path (the raider skiff).
    Corsair,
    /// Bureaucratic possess path (the appointment seal).
    Governor,
    /// Wreck-diver possess path (the derelict hulk).
    Salvager,
    /// Longshot-drive possess path (the junker shuttle).
    Improbable,
    /// Three-statutes automaton possess path (the cradle-pod).
    Lawbound,
    /// Walks-between-moments possess path (the moment-wound).
    Chronostalker,
    /// Anomalous-zone expedition possess path (the bloom scar).
    BloomTouched,
}

impl OriginKind {
    /// All origins in stable registration order.
    #[must_use]
    pub const fn all() -> [OriginKind; 8] {
        [
            OriginKind::Sovereign,
            OriginKind::Corsair,
            OriginKind::Governor,
            OriginKind::Salvager,
            OriginKind::Improbable,
            OriginKind::Lawbound,
            OriginKind::Chronostalker,
            OriginKind::BloomTouched,
        ]
    }

    /// Player-facing display name. Original names only — concept, never IP.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            OriginKind::Sovereign => "Fallen Sovereign",
            OriginKind::Corsair => "Corsair",
            OriginKind::Governor => "Planetary Governor",
            OriginKind::Salvager => "Salvager",
            OriginKind::Improbable => "Improbable Pilot",
            OriginKind::Lawbound => "Lawbound",
            OriginKind::Chronostalker => "Chronostalker",
            OriginKind::BloomTouched => "Bloom-Touched",
        }
    }

    /// Stable machine id, used in flags, STATS, and the roster summary.
    #[must_use]
    pub fn id(&self) -> &'static str {
        match self {
            OriginKind::Sovereign => "sovereign",
            OriginKind::Corsair => "corsair",
            OriginKind::Governor => "governor",
            OriginKind::Salvager => "salvager",
            OriginKind::Improbable => "pilot",
            OriginKind::Lawbound => "lawbound",
            OriginKind::Chronostalker => "chronostalker",
            OriginKind::BloomTouched => "bloom-touched",
        }
    }

    /// Parse a player-typed origin name: slugs, display names, and a few
    /// generic aliases, all case-insensitive.
    #[must_use]
    pub fn parse(s: &str) -> Option<OriginKind> {
        Some(match s.trim().to_lowercase().as_str() {
            "sovereign" | "fallen" | "fallen sovereign" | "monarch" | "exile" => {
                OriginKind::Sovereign
            }
            "corsair" | "pirate" | "raider" | "skiff" => OriginKind::Corsair,
            "governor" | "planetary governor" | "bureaucrat" => OriginKind::Governor,
            "salvager" | "salvage" | "wreck" | "wreck-diver" | "wreckdiver" => {
                OriginKind::Salvager
            }
            "pilot" | "improbable" | "improbable pilot" | "longshot" => OriginKind::Improbable,
            "lawbound" | "automaton" | "statutes" => OriginKind::Lawbound,
            "chronostalker" | "stalker" | "timewalker" => OriginKind::Chronostalker,
            "bloom" | "bloom-touched" | "bloom touched" | "touched" => OriginKind::BloomTouched,
            _ => return None,
        })
    }

    /// Attempt this origin's once-spawn (delegates to the origin module's
    /// own `spawn_*_once`, which keeps its once-guard).
    pub fn spawn(&self, world: &mut World) {
        match self {
            OriginKind::Sovereign => spawn_dented_crown_once(world),
            OriginKind::Corsair => spawn_raider_skiff_once(world),
            OriginKind::Governor => spawn_appointment_seal_once(world),
            OriginKind::Salvager => spawn_derelict_hulk_once(world),
            OriginKind::Improbable => spawn_shuttle_once(world),
            OriginKind::Lawbound => spawn_lawbound_cradle_once(world),
            OriginKind::Chronostalker => spawn_moment_wound_once(world),
            OriginKind::BloomTouched => spawn_bloom_scar_once(world),
        }
    }

    /// Whether this origin's once-spawn already attempted (so the director
    /// never schedules it twice).
    #[must_use]
    pub fn spawn_attempted(&self, world: &World) -> bool {
        match self {
            OriginKind::Sovereign => world
                .get_resource::<SovereignState>()
                .is_some_and(|s| s.crown_spawn_attempted),
            OriginKind::Corsair => world
                .get_resource::<CorsairState>()
                .is_some_and(|s| s.skiff_spawn_attempted),
            OriginKind::Governor => world
                .get_resource::<GovernorState>()
                .is_some_and(|s| s.seal_spawn_attempted),
            OriginKind::Salvager => world
                .get_resource::<SalvagerState>()
                .is_some_and(|s| s.hulk_spawn_attempted),
            OriginKind::Improbable => world
                .get_resource::<PilotState>()
                .is_some_and(|s| s.shuttle_spawn_attempted),
            OriginKind::Lawbound => world
                .get_resource::<LawboundState>()
                .is_some_and(|s| s.cradle_spawn_attempted),
            OriginKind::Chronostalker => world
                .get_resource::<ChronostalkerState>()
                .is_some_and(|s| s.wound_spawn_attempted),
            OriginKind::BloomTouched => world
                .get_resource::<BloomZone>()
                .is_some_and(|z| z.spawn_attempted),
        }
    }

    /// Chronicle flavor line fired when a trickled origin arrives
    /// mid-run. Original flavor, no lifted names or places.
    #[must_use]
    pub fn rumor(&self) -> &'static str {
        match self {
            OriginKind::Sovereign => {
                "Travelers speak of a dented crown changing hands beyond the ridge."
            }
            OriginKind::Corsair => {
                "A raider skiff limps into orbit, broadcasting a captain's writ."
            }
            OriginKind::Governor => {
                "A sealed appointment arrives by courier drone. Someone here is now officially in charge."
            }
            OriginKind::Salvager => "A derelict hulk drifts into sensor range, cold and dark.",
            OriginKind::Improbable => {
                "A junker shuttle drops out of the sky trailing impossible math."
            }
            OriginKind::Lawbound => "A cradle-pod touches down, humming three old statutes.",
            OriginKind::Chronostalker => "The moments near the wound have started walking.",
            OriginKind::BloomTouched => "The bloom has sent out a scout, and it is already wrong.",
        }
    }
}

/// The player's origin pick at game start. `None` = "surprise me"
/// (random roster).
#[derive(Resource, Debug, Default)]
pub struct OriginChoice {
    /// The chosen origin, if any.
    pub chosen: Option<OriginKind>,
}

/// One origin's planned arrival tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScheduledOrigin {
    /// Which origin.
    pub kind: OriginKind,
    /// Simulation tick at which the director fires its spawn.
    pub at_tick: u64,
}

/// Machine state for the origin roster: what's spawned, what's pending.
#[derive(Resource, Debug, Default)]
pub struct OriginSchedule {
    /// Whether the run's roster has been finalized (first director run).
    pub finalized: bool,
    /// Origins whose spawn already fired.
    pub spawned: Vec<OriginKind>,
    /// Origins still waiting for their arrival tick.
    pub pending: Vec<ScheduledOrigin>,
}

/// Plan this run's origin roster: always exactly 3 origins.
///
/// - The chosen origin (if any) spawns at tick 0.
/// - Otherwise two origins spawn at start, staggered at ticks 0 and 25
///   so their lander-ring site scans don't claim the same tiles in one
///   pass.
/// - The remaining origin(s) trickle in later at random ticks, so the
///   world keeps turning up new gameplay mid-run.
///
/// `rng` is caller-supplied so tests can seed it; the director passes a
/// thread RNG at runtime.
pub fn plan_origin_roster<R: Rng>(rng: &mut R, chosen: Option<OriginKind>) -> Vec<ScheduledOrigin> {
    let mut pool: Vec<OriginKind> = OriginKind::all()
        .into_iter()
        .filter(|k| Some(*k) != chosen)
        .collect();
    pool.shuffle(rng);
    let mut plan = Vec::with_capacity(3);
    match chosen {
        Some(kind) => {
            // The player's pick is always there at tick 0; two more
            // origins trickle in later so the mid-game stays fresh.
            plan.push(ScheduledOrigin { kind, at_tick: 0 });
            let mut trickle_tick = 150u64;
            for k in pool.into_iter().take(2) {
                let jitter = rng.gen_range(0..=150u64);
                plan.push(ScheduledOrigin {
                    kind: k,
                    at_tick: trickle_tick + jitter,
                });
                trickle_tick += 250;
            }
        }
        None => {
            // Two at start (staggered so their lander-ring site scans
            // don't claim the same tiles in one pass), one trickling in
            // later.
            for (i, k) in pool.into_iter().take(3).enumerate() {
                let at_tick = match i {
                    0 => 0,
                    1 => 25,
                    _ => 200 + rng.gen_range(0..=600u64),
                };
                plan.push(ScheduledOrigin { kind: k, at_tick });
            }
        }
    }
    plan
}

/// Exclusive system: finalizes the run's roster once, then fires any
/// origin spawns whose arrival tick has come. Replaces the eight
/// unconditional `spawn_*_once` schedule entries.
///
/// Must run before the origin tick systems and before the lawbound /
/// chronostalker sabotage bridges (they read origin state resources).
/// Make sure every origin's state resources exist, whether or not the
/// origin is in this run's roster.
///
/// The per-origin tick systems self-ensure, but the sabotage/crisis
/// bridges take `ResMut<OriginState>` as a direct system parameter, so
/// Bevy panics if the resource is missing. The old unconditional
/// `spawn_*_once` entries created the resources as a side effect of
/// their own ensure calls; the director must preserve that invariant.
pub fn ensure_all_origin_resources(world: &mut World) {
    ensure_sovereign_resources(world);
    ensure_corsair_resources(world);
    ensure_governor_resources(world);
    ensure_salvager_resources(world);
    ensure_pilot_resources(world);
    ensure_lawbound_resources(world);
    ensure_chronostalker_resources(world);
    ensure_bloomtouched_resources(world);
}

pub fn origin_spawn_director(world: &mut World) {
    world.init_resource::<OriginChoice>();
    world.init_resource::<OriginSchedule>();
    if !world.resource::<OriginSchedule>().finalized {
        // Resources first: the bridges below read origin state directly
        // and must never see a missing resource, even for origins that
        // never spawn this run.
        ensure_all_origin_resources(world);
        let chosen = world.resource::<OriginChoice>().chosen;
        let mut rng = rand::thread_rng();
        let plan = plan_origin_roster(&mut rng, chosen);
        let mut schedule = world.resource_mut::<OriginSchedule>();
        schedule.pending = plan;
        schedule.finalized = true;
    }
    let tick = world.get_resource::<SimulationTime>().map_or(0, |t| t.tick);
    let due: Vec<(OriginKind, u64)> = {
        let schedule = world.resource::<OriginSchedule>();
        schedule
            .pending
            .iter()
            .filter(|p| p.at_tick <= tick)
            .map(|p| (p.kind, p.at_tick))
            .collect()
    };
    for (kind, at_tick) in due {
        kind.spawn(world);
        let mut schedule = world.resource_mut::<OriginSchedule>();
        schedule.pending.retain(|p| p.kind != kind);
        if !schedule.spawned.contains(&kind) {
            schedule.spawned.push(kind);
        }
        // Trickled arrivals (anything past tick 0) announce themselves in
        // the chronicle so the player notices the world turning.
        if at_tick > 0 {
            world.init_resource::<Events<AddChronicleEvent>>();
            world
                .resource_mut::<Events<AddChronicleEvent>>()
                .send(AddChronicleEvent {
                    text: kind.rumor().to_string(),
                    importance: EventImportance::Standard,
                });
        }
    }
}

/// Compact roster summary for the headless `origins` command and STATS,
/// e.g. `sovereign,corsair|pending:1` or `undecided` before the first tick.
#[must_use]
pub fn origin_roster_summary(world: &World) -> String {
    let Some(schedule) = world.get_resource::<OriginSchedule>() else {
        return "undecided".to_string();
    };
    if !schedule.finalized {
        return "undecided".to_string();
    }
    let spawned: Vec<&str> = schedule.spawned.iter().map(OriginKind::id).collect();
    format!(
        "{}|pending:{}",
        spawned.join(","),
        schedule.pending.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    fn seeded(seed: u64) -> StdRng {
        StdRng::seed_from_u64(seed)
    }

    #[test]
    fn origin_all_has_eight_stable_order() {
        let all = OriginKind::all();
        assert_eq!(all.len(), 8);
        assert_eq!(
            all,
            [
                OriginKind::Sovereign,
                OriginKind::Corsair,
                OriginKind::Governor,
                OriginKind::Salvager,
                OriginKind::Improbable,
                OriginKind::Lawbound,
                OriginKind::Chronostalker,
                OriginKind::BloomTouched,
            ]
        );
    }

    #[test]
    fn origin_names_are_original_display_names() {
        let names: Vec<&str> = OriginKind::all().iter().map(OriginKind::name).collect();
        assert_eq!(
            names,
            [
                "Fallen Sovereign",
                "Corsair",
                "Planetary Governor",
                "Salvager",
                "Improbable Pilot",
                "Lawbound",
                "Chronostalker",
                "Bloom-Touched",
            ]
        );
        let ids: Vec<&str> = OriginKind::all().iter().map(OriginKind::id).collect();
        assert_eq!(
            ids,
            [
                "sovereign",
                "corsair",
                "governor",
                "salvager",
                "pilot",
                "lawbound",
                "chronostalker",
                "bloom-touched",
            ]
        );
    }

    #[test]
    fn parse_accepts_slugs_names_and_aliases() {
        for (input, want) in [
            ("sovereign", OriginKind::Sovereign),
            ("Fallen Sovereign", OriginKind::Sovereign),
            ("corsair", OriginKind::Corsair),
            ("pirate", OriginKind::Corsair),
            ("governor", OriginKind::Governor),
            ("salvager", OriginKind::Salvager),
            ("PILOT", OriginKind::Improbable),
            ("improbable", OriginKind::Improbable),
            ("lawbound", OriginKind::Lawbound),
            ("chronostalker", OriginKind::Chronostalker),
            ("bloom-touched", OriginKind::BloomTouched),
            ("bloom", OriginKind::BloomTouched),
        ] {
            assert_eq!(OriginKind::parse(input), Some(want), "input: {input}");
        }
    }

    #[test]
    fn parse_rejects_unknown_and_empty() {
        for input in ["", "   ", "zzz-no-origin", "windsor", "sad king billy"] {
            assert_eq!(OriginKind::parse(input), None, "input: {input}");
        }
    }

    #[test]
    fn roster_no_choice_has_three_unique() {
        let plan = plan_origin_roster(&mut seeded(7), None);
        assert_eq!(plan.len(), 3, "plan: {plan:?}");
        let mut kinds: Vec<OriginKind> = plan.iter().map(|p| p.kind).collect();
        kinds.sort_by_key(|k| *k as u8);
        kinds.dedup();
        assert_eq!(kinds.len(), 3, "duplicate origins in plan: {plan:?}");
    }

    #[test]
    fn roster_no_choice_starts_two_staggered() {
        let plan = plan_origin_roster(&mut seeded(7), None);
        assert_eq!(plan[0].at_tick, 0, "first origin must spawn at tick 0");
        assert_eq!(plan[1].at_tick, 25, "second origin must be staggered");
    }

    #[test]
    fn roster_no_choice_third_trickles_later() {
        let plan = plan_origin_roster(&mut seeded(7), None);
        assert!(
            (200..=800).contains(&plan[2].at_tick),
            "trickle tick out of range: {}",
            plan[2].at_tick
        );
    }

    #[test]
    fn roster_chosen_spawns_at_tick_zero() {
        let plan = plan_origin_roster(&mut seeded(7), Some(OriginKind::Lawbound));
        assert_eq!(plan.len(), 3);
        assert_eq!(plan[0].kind, OriginKind::Lawbound);
        assert_eq!(plan[0].at_tick, 0);
        // The chosen origin appears exactly once.
        assert_eq!(
            plan.iter().filter(|p| p.kind == OriginKind::Lawbound).count(),
            1
        );
        // The other two trickle in later.
        assert!(plan[1].at_tick >= 150);
        assert!(plan[2].at_tick >= 150);
        assert_ne!(plan[1].kind, OriginKind::Lawbound);
        assert_ne!(plan[2].kind, OriginKind::Lawbound);
    }

    #[test]
    fn roster_seeded_determinism() {
        let a = plan_origin_roster(&mut seeded(42), None);
        let b = plan_origin_roster(&mut seeded(42), None);
        assert_eq!(a, b);
        let c = plan_origin_roster(&mut seeded(42), Some(OriginKind::Corsair));
        let d = plan_origin_roster(&mut seeded(42), Some(OriginKind::Corsair));
        assert_eq!(c, d);
    }

    #[test]
    fn roster_randomizes_across_seeds() {
        // Over 30 seeds the first-spawned origin should vary a lot; if the
        // "random" roster were secretly deterministic we'd see 1 distinct.
        let mut firsts = std::collections::HashSet::new();
        for seed in 0..30u64 {
            let plan = plan_origin_roster(&mut seeded(seed), None);
            firsts.insert(plan[0].kind);
        }
        assert!(
            firsts.len() >= 4,
            "roster looks fixed across seeds: {firsts:?}"
        );
    }

    fn minimal_world() -> World {
        let mut world = World::new();
        world.insert_resource(SimulationTime::default());
        world.insert_resource(Events::<AddChronicleEvent>::default());
        world
    }

    #[test]
    fn director_sends_rumor_chronicle_for_trickled_arrival() {
        let mut world = minimal_world();
        world.insert_resource(OriginSchedule {
            finalized: true,
            spawned: Vec::new(),
            pending: vec![
                ScheduledOrigin {
                    kind: OriginKind::Salvager,
                    at_tick: 100,
                },
                ScheduledOrigin {
                    kind: OriginKind::Corsair,
                    at_tick: 0,
                },
            ],
        });
        world.resource_mut::<SimulationTime>().tick = 150;
        origin_spawn_director(&mut world);

        let events = world.resource::<Events<AddChronicleEvent>>();
        let mut reader = events.get_reader();
        let texts: Vec<String> = reader.read(&events).map(|e| e.text.clone()).collect();
        // The trickled salvager announces itself...
        assert!(
            texts.iter().any(|t| t.contains("derelict hulk")),
            "trickle rumor missing: {texts:?}"
        );
        // ...but the tick-0 corsair spawns quietly (it was there from the start).
        assert!(
            !texts.iter().any(|t| t.contains("raider skiff")),
            "tick-0 spawn should not rumor: {texts:?}"
        );
    }

    #[test]
    fn director_ensures_all_origin_resources_even_when_unspawned() {
        // Regression: the sabotage/crisis bridges take ResMut<OriginState>
        // directly, so every origin's resources must exist even when the
        // origin never spawns this run.
        let mut world = minimal_world();
        world.insert_resource(OriginChoice {
            chosen: Some(OriginKind::Sovereign),
        });
        origin_spawn_director(&mut world);
        assert!(world.get_resource::<SovereignState>().is_some());
        assert!(world.get_resource::<CorsairState>().is_some());
        assert!(world.get_resource::<GovernorState>().is_some());
        assert!(world.get_resource::<SalvagerState>().is_some());
        assert!(world.get_resource::<PilotState>().is_some());
        assert!(world.get_resource::<LawboundState>().is_some());
        assert!(world.get_resource::<ChronostalkerState>().is_some());
        assert!(world.get_resource::<BloomZone>().is_some());
    }

    #[test]
    fn director_finalizes_and_fires_tick_zero_spawn() {
        let mut world = minimal_world();
        // No OriginChoice resource: the director must default to "surprise me".
        origin_spawn_director(&mut world);
        let sched = world.resource::<OriginSchedule>();
        assert!(sched.finalized, "director must finalize the roster");
        assert_eq!(sched.spawned.len(), 1, "only the tick-0 spawn fires");
        assert_eq!(sched.pending.len(), 2);
        assert_eq!(sched.pending[0].at_tick, 25);
        // The fired origin's once-guard is marked attempted.
        assert!(sched.spawned[0].spawn_attempted(&world));
    }

    #[test]
    fn director_defers_trickle_spawn_until_its_tick() {
        let mut world = minimal_world();
        world.insert_resource(OriginSchedule {
            finalized: true,
            spawned: Vec::new(),
            pending: vec![ScheduledOrigin {
                kind: OriginKind::Corsair,
                at_tick: 100,
            }],
        });
        // Tick 0: nothing due.
        origin_spawn_director(&mut world);
        assert!(!OriginKind::Corsair.spawn_attempted(&world));
        assert!(world.resource::<OriginSchedule>().spawned.is_empty());

        // Tick 150: the trickle fires.
        world.resource_mut::<SimulationTime>().tick = 150;
        origin_spawn_director(&mut world);
        assert!(OriginKind::Corsair.spawn_attempted(&world));
        let sched = world.resource::<OriginSchedule>();
        assert_eq!(sched.spawned, vec![OriginKind::Corsair]);
        assert!(sched.pending.is_empty());
    }

    #[test]
    fn director_never_double_spawns() {
        let mut world = minimal_world();
        world.insert_resource(OriginChoice {
            chosen: Some(OriginKind::Governor),
        });
        world.resource_mut::<SimulationTime>().tick = 10_000;
        origin_spawn_director(&mut world);
        let first = world.resource::<OriginSchedule>().spawned.clone();
        assert_eq!(first.len(), 3, "all three fire by tick 10000");
        origin_spawn_director(&mut world);
        origin_spawn_director(&mut world);
        let sched = world.resource::<OriginSchedule>();
        assert_eq!(sched.spawned, first, "re-runs must not re-fire");
        assert!(sched.pending.is_empty());
    }

    #[test]
    fn origin_choice_defaults_to_none() {
        assert_eq!(OriginChoice::default().chosen, None);
    }

    #[test]
    fn origin_status_mentions_spawned_ids() {
        let mut world = minimal_world();
        world.insert_resource(OriginSchedule {
            finalized: true,
            spawned: vec![OriginKind::Sovereign, OriginKind::Corsair],
            pending: vec![ScheduledOrigin {
                kind: OriginKind::BloomTouched,
                at_tick: 500,
            }],
        });
        let summary = origin_roster_summary(&world);
        assert!(summary.contains("sovereign"), "got: {summary}");
        assert!(summary.contains("corsair"), "got: {summary}");
        assert!(summary.contains("pending:1"), "got: {summary}");

        let fresh = minimal_world();
        assert_eq!(origin_roster_summary(&fresh), "undecided");
    }

    #[test]
    fn rumor_lines_are_nonempty_and_original() {
        for kind in OriginKind::all() {
            let rumor = kind.rumor();
            assert!(!rumor.is_empty(), "{kind:?} has an empty rumor line");
            assert!(rumor.len() < 200, "{kind:?} rumor too long: {rumor}");
        }
    }

    // --- IP guard ----------------------------------------------------------

    fn public_strings() -> Vec<String> {
        let mut out = Vec::new();
        for kind in OriginKind::all() {
            out.push(kind.name().to_string());
            out.push(kind.id().to_string());
            out.push(kind.rumor().to_string());
        }
        out
    }

    #[test]
    fn ip_guard_no_banned_terms() {
        // Union of the banned-term lists from the origin IP-guard tests
        // (Simmons / Adams / Asimov / VanderMeer lanes) plus the standing
        // Sad-King-Billy rule: concept only, never lifted names/places.
        let banned = [
            // Simmons lane
            "shrike",
            "hyperion",
            "ouster",
            "templars",
            // Adams lane
            "hitchhiker",
            "vogon",
            "zaphod",
            "ford prefect",
            "marvin",
            "dont panic",
            "don't panic",
            "babel fish",
            "infinite improbability",
            // Asimov lane
            "asimov",
            "psychohistory",
            "hari seldon",
            "foundation",
            "spacer",
            // VanderMeer lane
            "annihilation",
            "area x",
            "southern reach",
            "vandermeer",
            // Standing rules
            "sad king billy",
            "windsor",
            "windsor-in-exile",
        ];
        for s in public_strings() {
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
