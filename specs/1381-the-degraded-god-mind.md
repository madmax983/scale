# 1381: The Degraded God-Mind

## 1. Overview

A civilization can immortalize a dying leader as an uploaded AI ruler,
granting a permanent stability bonus and immunity to succession crises. But
over centuries the code succumbs to bit-rot: the God-Mind issues erratic
edicts, demands bizarre resource tributes (pave a functioning agri-world for
a monument to a pet that died 400 years ago), and declares corrupted-memory
wars on fallen empires. Endure the escalating insanity to keep the bonus, or
unplug the Eternal Ruler and risk catastrophic civil war.

## 2. Dependencies

- `src/layer3/diplomacy/succession.rs` (`Faction`, `CurrentLeader`, `Leader`,
  `Age`, `Dead`, `SuccessionCrisis`, `SuccessionEvent`)
- `src/layer1/core/chronicle.rs` (`AddChronicleEvent`)
- Diplomacy war/relations systems (consume `GodMindWarDeclared`)

## 3. RED Phase: Tests First

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::*;
    use crate::layer3::diplomacy::succession::{Faction, Leader};

    fn spawn_faction(world: &mut World, name: &str) -> Entity {
        let leader = world.spawn(Leader { name: "Founder Vex".into() }).id();
        world
            .spawn((
                Faction { name: name.into() },
                crate::layer3::diplomacy::succession::CurrentLeader(leader),
            ))
            .id()
    }

    #[test]
    fn test_upload_grants_stability_and_ends_succession() {
        let mut world = World::new();
        let faction = spawn_faction(&mut world, "Vex Hegemony");

        upload_god_mind(&mut world, faction, "Founder Vex");

        let mind = world.get::<GodMind>(faction).unwrap();
        assert_eq!(mind.bit_rot, 0.0, "a fresh upload starts pristine");
        assert!(mind.stability_bonus > 0.0, "upload grants a stability bonus");
        assert!(
            world.get::<crate::layer3::diplomacy::succession::SuccessionCrisis>(faction).is_none(),
            "the Eternal Ruler never triggers succession crises"
        );
    }

    #[test]
    fn test_bit_rot_accumulates_over_centuries() {
        let mut world = World::new();
        let faction = spawn_faction(&mut world, "Vex Hegemony");
        upload_god_mind(&mut world, faction, "Founder Vex");

        for _ in 0..CENTURIES_TO_FULL_ROT {
            advance_bit_rot(&mut world);
        }

        let mind = world.get::<GodMind>(faction).unwrap();
        assert!(
            (mind.bit_rot - 1.0).abs() < f32::EPSILON,
            "bit-rot should saturate after enough centuries"
        );
    }

    #[test]
    fn test_edict_severity_escalates_with_rot() {
        let mut world = World::new();
        world.insert_resource(Events::<GodMindEdict>::default());
        let faction = spawn_faction(&mut world, "Vex Hegemony");
        upload_god_mind(&mut world, faction, "Founder Vex");
        world.get_mut::<GodMind>(faction).unwrap().bit_rot = 0.9;

        issue_god_mind_edict(&mut world, faction, 0.9);

        let events = world.resource::<Events<GodMindEdict>>();
        let mut cursor = events.get_cursor();
        let edicts: Vec<_> = cursor.read(events).collect();
        assert!(!edicts.is_empty(), "a rotten god-mind should issue edicts");
        assert!(
            edicts.iter().any(|e| matches!(
                e.kind,
                EdictKind::BizarreTribute { .. } | EdictKind::CorruptedMemoryWar { .. }
            )),
            "high rot should produce severe (tribute/war) edicts"
        );
    }

    #[test]
    fn test_unplugging_risks_civil_war() {
        let mut world = World::new();
        world.insert_resource(Events::<GodMindSchism>::default());
        let faction = spawn_faction(&mut world, "Vex Hegemony");
        upload_god_mind(&mut world, faction, "Founder Vex");
        // Let it rule long enough that pops worship it.
        world.get_mut::<GodMind>(faction).unwrap().reign_ticks = LONG_REIGN;

        unplug_god_mind(&mut world, faction);

        assert!(
            world.get::<GodMind>(faction).is_none(),
            "unplugging removes the Eternal Ruler"
        );
        let events = world.resource::<Events<GodMindSchism>>();
        let mut cursor = events.get_cursor();
        let schisms: Vec<_> = cursor.read(events).collect();
        assert_eq!(schisms.len(), 1, "unplugging fires one schism event");
        assert!(
            schisms[0].civil_war_risk > 0.5,
            "a long-reigning god-mind's removal should risk civil war"
        );
    }
}
```

## 4. GREEN Phase: Minimal Implementation

```rust
// The SIMPLEST code that makes tests pass.
// - GodMind: Component on the Faction entity {
//     leader_name: String, uploaded_tick: u64, bit_rot: f32 (0..=1),
//     reign_ticks: u64, stability_bonus: f32 }.
// - upload_god_mind(world, faction, name): attaches GodMind (bit_rot 0,
//   reign 0, STABILITY_BONUS), despawns/retires the mortal Leader entity,
//   emits chronicle event ("Founder Vex will never die. The Eternal Ruler
//   ascends.").
// - advance_bit_rot: every ROT_TICK_INTERVAL ticks, for each GodMind:
//   bit_rot = (bit_rot + ROT_PER_CENTURY).min(1.0); reign_ticks += interval.
// - issue_god_mind_edict: per-tick probability p = EDICT_BASE_RATE * bit_rot;
//   on fire, pick severity by bit_rot: low rot -> Erratic (flavor + minor
//   inefficiency modifier), high rot -> BizarreTribute { resource, amount,
//   monument: bool } or CorruptedMemoryWar { target: fallen faction }.
//   Emits GodMindEdict event; tribute applies a resource sink to the faction.
// - CorruptedMemoryWar picks targets from factions marked fallen/dead — the
//   god-mind remembers enemies that no longer exist and attacks their
//   successors or empty space (resolved as a real war declaration event the
//   diplomacy layer consumes).
// - unplug_god_mind: removes GodMind, emits GodMindSchism {
//   civil_war_risk = f(reign_ticks) }, which the succession/crisis systems
//   consume like a SuccessionCrisis with the given risk weight.
```

## 5. REFACTOR Phase: Quality & Design

- Emit `AddChronicleEvent` for: upload, first erratic edict ("The Eternal
  Ruler spoke in static today."), first bizarre tribute, first
  corrupted-memory war, and unplugging.
- Tribute monuments: a bizarre tribute with `monument: true` should spawn a
  real (useless, upkeep-costing) monument entity on the target world — the
  pet memorial must be *visible*.
- Scale `stability_bonus` against faction size so the bonus stays tempting
  even in the late game when the rot is total.
- Lore hook: god-mind edict texts feed the procedural generator under
  fragment tags `godmind-edict` / `godmind-tribute`; the corrupted memories
  reference real dead leaders from the chronicle.

## 6. Acceptance Criteria (Testable!)

- [ ] All tests in RED phase pass
- [ ] Uploading grants stability and suppresses succession crises
- [ ] Bit-rot saturates over centuries; edict frequency and severity scale with it
- [ ] A corrupted-memory war can fire against a fallen faction
- [ ] Unplugging after a long reign produces a high civil-war-risk schism
- [ ] `cargo test` returns 0 failures
- [ ] `cargo clippy -- -D warnings` passes

## 7. Technical Guidance

- Hook the upload into the existing succession flow: when a `Leader` would
  die of `Age`, offer/trigger `upload_god_mind` as an alternative to
  `process_succession_system` (a player or faction-AI decision point, not an
  automatic override — the *choice* is the feature).
- Keep `GodMind` on the `Faction` entity, not the leader entity, so the
  mortal leader's death doesn't take the upload with it.
- Edicts are events consumed by economy/diplomacy systems — this spec does
  NOT reimplement war or resource transfer, it only emits and sinks.
- One session means: upload + rot + edicts + tribute + war event + unplug +
  schism. Monument rendering, faction-AI upload decisions, and god-mind
  "personalities" are follow-up specs, not this one.

## 8. Questions

*Builder: add questions here if spec is unclear.*
