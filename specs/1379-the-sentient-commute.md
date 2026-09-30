# 1379: The Sentient Commute

## 1. Overview

Heavily-used path tiles accumulate "Desire Dust," which slightly speeds pop
movement along them. Past a threshold of use, an overused road coagulates into
a rudimentary immobile intelligence — a `RoadMind` — that emits pheromones
compelling pops to route through it even when better paths exist. Deleting the
road doesn't help: pops keep walking the ghost of the route until the player
excavates the dust tile by tile. Organic efficiency vs. stubborn permanence.

## 2. Dependencies

- `src/layer1/execution/movement.rs` (`movement_system`, `Speed`)
- `src/layer1/entities/pop.rs` (`Speed { base, current, accumulator }`,
  `reset_speed_system`)
- `src/layer1/core/chronicle.rs` (`AddChronicleEvent`)
- Designation system (`DesignationType`) for the excavate counterplay

## 3. RED Phase: Tests First

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::*;
    use crate::layer1::entities::pop::{Pop, Speed};
    use crate::layer1::map::GridPosition;

    #[test]
    fn test_foot_traffic_accumulates_desire_dust() {
        let mut world = World::new();
        world.insert_resource(DesireDust::default());
        let pop = world.spawn((Pop, GridPosition { x: 3, y: 3 })).id();

        // Simulate one tick of a moving pop standing on (3, 3).
        accumulate_desire_dust(&mut world);

        let dust = world.resource::<DesireDust>();
        assert!(
            dust.amount_at(3, 3) > 0.0,
            "foot traffic should deposit Desire Dust"
        );
        let _ = pop;
    }

    #[test]
    fn test_dusty_tiles_grant_speed_bonus() {
        let mut world = World::new();
        let mut dust = DesireDust::default();
        dust.deposit(5, 5, SENTIENCE_THRESHOLD); // well-worn road
        world.insert_resource(dust);
        let pop = world.spawn((
            Pop,
            GridPosition { x: 5, y: 5 },
            Speed { base: 1.0, current: 1.0, accumulator: 0.0 },
        )).id();

        apply_desire_dust_speed(&mut world);

        let speed = world.get::<Speed>(pop).unwrap();
        assert!(
            speed.current > speed.base,
            "pops on dusty tiles should move faster"
        );
    }

    #[test]
    fn test_road_mind_coagulates_past_threshold() {
        let mut world = World::new();
        let mut dust = DesireDust::default();
        // Paint a connected stretch of heavy-use tiles.
        for x in 0..6 {
            dust.deposit(x, 0, SENTIENCE_THRESHOLD / 4.0);
        }
        world.insert_resource(dust);

        check_road_sentience(&mut world);

        let minds = world.query::<&RoadMind>().iter(&world).count();
        assert_eq!(minds, 1, "a saturated road should coagulate one RoadMind");
    }

    #[test]
    fn test_excavation_clears_dust_and_kills_road_mind() {
        let mut world = World::new();
        let mut dust = DesireDust::default();
        dust.deposit(2, 2, SENTIENCE_THRESHOLD);
        world.insert_resource(dust);
        let mind = world.spawn((RoadMind { tiles: vec![(2, 2)] },)).id();

        excavate_dust_tile(&mut world, 2, 2);

        assert_eq!(world.resource::<DesireDust>().amount_at(2, 2), 0.0);
        assert!(
            world.get_entity(mind).is_err(),
            "a RoadMind with no dusty tiles should dissolve"
        );
    }
}
```

## 4. GREEN Phase: Minimal Implementation

```rust
// The SIMPLEST code that makes tests pass.
// - DesireDust: Resource wrapping HashMap<(i32, i32), f32> with
//   deposit()/amount_at(), plus slow per-tick decay.
// - accumulate_desire_dust: for each Pop with a MovementTarget (i.e. actually
//   travelling), deposit DUST_PER_TICK on its GridPosition tile.
// - apply_desire_dust_speed: after reset_speed_system, for each Pop standing
//   on a dusty tile, speed.current = base * (1.0 + DUST_SPEED_BONUS * dust).
// - check_road_sentience: flood-fill connected dusty tiles; if a region's
//   total dust >= SENTIENCE_THRESHOLD, spawn one RoadMind entity for it.
// - RoadMind: immobile marker with its tile list; while alive, pops whose
//   MovementTarget lies within PHEROMONE_RADIUS get their target rerouted
//   through the road's tiles (compelled routing) — implement as a bias in
//   the existing movement target selection, not a pathfinder rewrite.
// - excavate_dust_tile: clears dust on one tile (wired to a new
//   DesignationType::ExcavateDust work order); dissolves RoadMinds whose
//   tile list is fully excavated.
```

## 5. REFACTOR Phase: Quality & Design

- Emit `AddChronicleEvent` when the first RoadMind coagulates
  ("The old north road has begun to dream.") and when one is excavated.
- Cap the speed bonus so dust highways don't break balance
  (`DUST_SPEED_BONUS_MAX`).
- Make dust decay very slow (roads are remembered for a long time — that's
  the point) but non-zero so abandoned ghost roads eventually fade.
- Lore hook: LEXICON term "the Broadcast / the Sell" is taken; file the
  road-mind's whispers under a new fragment tag `desire-dust` for the
  procedural text generator.

## 6. Acceptance Criteria (Testable!)

- [ ] All tests in RED phase pass
- [ ] Pops visibly prefer dusty tiles (speed bonus measurable in headless stats)
- [ ] A RoadMind appears after sustained heavy traffic and compels rerouting
- [ ] Excavating all of a RoadMind's tiles dissolves it
- [ ] `cargo test` returns 0 failures
- [ ] `cargo clippy -- -D warnings` passes

## 7. Technical Guidance

- Mirror `PressureGrid`'s resource-plus-system pattern; do NOT make tiles
  entities.
- Run `apply_desire_dust_speed` after `reset_speed_system` so the bonus
  isn't wiped each tick.
- Keep the compelled-routing bias small and probabilistic per tick — a hard
  override will look like a pathfinding bug.
- One session means: dust + bonus + sentience + excavation. Pheromone
  visuals and road-mind "demands" are follow-up specs, not this one.

## 8. Questions

*Builder: add questions here if spec is unclear.*
