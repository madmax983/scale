# 1380: The Propaganda Constellation

## 1. Overview

Launch slogan satellites that link into a giant glowing message in the night
sky, granting a colony-wide morale/productivity boost that overrides negative
moods. Rival factions can hack the constellation to flip the message into
despair (morale penalty instead). The counterplay is shooting down your own
satellites — which kills the message but feeds the existing orbital-debris
mechanic. Unparalleled mood control vs. hacking and Kessler-syndrome
vulnerability.

## 2. Dependencies

- `src/layer2/debris.rs` (`OrbitalDebris` — shoot-downs feed it)
- `src/layer2/events.rs` (`LaunchEvent` — satellite launches hook in here)
- `src/layer2/fleet.rs` (`InOrbit` — satellites are orbital entities)
- `src/layer1/social/morale.rs` (`Morale { value, modifiers }`)
- `src/layer1/core/chronicle.rs` (`AddChronicleEvent`)

## 3. RED Phase: Tests First

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::*;
    use crate::layer1::social::morale::{Morale, MoodModifier};
    use crate::layer2::debris::OrbitalDebris;

    fn spawn_pop(world: &mut World) -> Entity {
        world
            .spawn((crate::layer1::pop::Pop, Morale::default()))
            .id()
    }

    #[test]
    fn test_linked_satellites_form_constellation() {
        let mut world = World::new();
        world.insert_resource(Constellation::default());
        for _ in 0..CONSTELLATION_MIN_SATELLITES {
            world.spawn((SloganSatellite::default(),));
        }

        update_constellation(&mut world);

        let constellation = world.resource::<Constellation>();
        assert!(
            constellation.active(),
            "enough linked satellites should activate the constellation"
        );
    }

    #[test]
    fn test_active_constellation_boosts_colony_morale() {
        let mut world = World::new();
        let mut constellation = Constellation::default();
        constellation.message = SloganMessage::Hope("WE ENDURE".to_string());
        world.insert_resource(constellation);
        let pop = spawn_pop(&mut world);

        apply_constellation_morale(&mut world);

        let morale = world.get::<Morale>(pop).unwrap();
        assert!(
            morale.modifiers.iter().any(|m| m.label == "Propaganda Constellation"),
            "active constellation should add a morale modifier to every pop"
        );
    }

    #[test]
    fn test_hack_flips_message_to_despair() {
        let mut world = World::new();
        let mut constellation = Constellation::default();
        constellation.message = SloganMessage::Hope("WE ENDURE".to_string());
        world.insert_resource(constellation);

        hack_constellation(&mut world, "OBEY THE STATIC".to_string());

        let constellation = world.resource::<Constellation>();
        assert!(
            matches!(constellation.message, SloganMessage::Despair(_)),
            "a successful hack should flip the slogan to despair"
        );
    }

    #[test]
    fn test_shooting_down_satellite_kills_boost_and_creates_debris() {
        let mut world = World::new();
        world.insert_resource(OrbitalDebris::default());
        let sat = world.spawn((SloganSatellite::default(),)).id();

        shoot_down_satellite(&mut world, sat);

        assert!(world.get_entity(sat).is_err(), "satellite should be destroyed");
        assert!(
            world.resource::<OrbitalDebris>().0 > 0.0,
            "shoot-down should feed the orbital debris mechanic"
        );
    }
}
```

## 4. GREEN Phase: Minimal Implementation

```rust
// The SIMPLEST code that makes tests pass.
// - SloganSatellite: Component on an orbital entity (spawned via a colony
//   "Launch Slogan Satellite" action that emits LaunchEvent; costs resources).
// - Constellation: Resource { satellites: Vec<Entity>, message: SloganMessage }.
//   active() == satellites.len() >= CONSTELLATION_MIN_SATELLITES.
// - update_constellation: reconcile the satellite list (despawned sats drop
//   out); flip active/inactive.
// - apply_constellation_morale: while active, push/refresh a
//   MoodModifier { label: "Propaganda Constellation", value: +HOPE_BOOST }
//   (or -DESPAIR_PENALTY when hacked) on every Pop's Morale each tick.
//   The modifier's magnitude is tuned to override typical negative moods.
// - hack_constellation: rival-faction event hook; on success replaces
//   message with SloganMessage::Despair(String). Hack attempts tick while
//   active with probability scaled by active rival count.
// - shoot_down_satellite: destroys the satellite entity, adds
//   SHOOT_DOWN_DEBRIS to the OrbitalDebris resource, emits a chronicle event.
```

## 5. REFACTOR Phase: Quality & Design

- Emit `AddChronicleEvent` on first activation ("The sky learned our name."),
  on hack ("The sky is lying to us."), and on shoot-down.
- Night-sky visibility: the constellation only applies its modifier during
  the planet's night cycle (hook `day_night.rs`) — the slogan must be
  *visible* to work.
- Lore hook: slogan texts drawn from the procedural generator; tag fragments
  `slogan-hope` / `slogan-despair`. LEXICON's "the Broadcast / the Sell"
  ("The Broadcast tells us what we need.") is the in-fiction name pops use
  for the constellation.
- Balance: debris from shoot-downs must be meaningful against the existing
  Kessler thresholds in `debris.rs` — killing a hijacked constellation
  should hurt.

## 6. Acceptance Criteria (Testable!)

- [ ] All tests in RED phase pass
- [ ] Launching N satellites activates the boost; dropping below N ends it
- [ ] A hack flips hope to despair (morale penalty visible in headless stats)
- [ ] Shooting down your own satellites ends a hijacked message and raises debris
- [ ] `cargo test` returns 0 failures
- [ ] `cargo clippy -- -D warnings` passes

## 7. Technical Guidance

- Satellites are entities with `InOrbit`, not buildings — reuse the fleet
  orbital patterns, don't invent a new orbital model.
- The morale modifier must be refreshed (not stacked) each tick: remove the
  old "Propaganda Constellation" modifier before pushing the new one.
- Hack probability per tick should be low; the dread comes from the flip,
  not the frequency.
- One session means: launch + link + boost + hack + shoot-down + debris.
  Rival-faction AI that *plans* hacks, and ground-based uplink defenses, are
  follow-up specs, not this one.

## 8. Questions

*Builder: add questions here if spec is unclear.*

**Builder note (2026-10-03, implemented):** RED test 2 (`test_active_constellation_boosts_colony_morale`) sets a Hope message on a `Constellation::default()` with *zero* satellites and expects the morale modifier — while §6 requires "dropping below N ends the boost." Reconciliation: added `SloganMessage::Dark`. `update_constellation` darkens the sky whenever the link drops below `CONSTELLATION_MIN_SATELLITES` (killing hope *or* hacked despair), and `apply_constellation_morale` broadcasts only while a message is lit. Observable game behavior is exactly "while active"; all four RED tests pass unmodified (test 2 passes because it sets a lit Hope message directly). Two extra tests cover refresh-not-stack and below-threshold deactivation.
