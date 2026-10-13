use bevy::app::App;

use scale::layer1::social::emotional_contagion::{
    trigger_emotional_contagion_system, ContagionType, EmotionalContagion,
};
use scale::layer1::social::morale::Morale;

// Spec 346 adaptation: the original draft of this test used the sketch
// API (ContagionType::Terror, radius: f32, Morale on a 0-100 scale). The
// real Morale component is clamped to 0.0-1.0 by update_morale_cache_system
// and Spec 346 names the low extreme Terror, so the values/variant below
// are the same three assertions on the real scale.

#[test]
fn test_emotional_contagion_triggers_terror() {
    let mut app = App::new();
    app.add_plugins(bevy::MinimalPlugins);
    app.add_systems(bevy::app::Update, trigger_emotional_contagion_system);

    let pop = app
        .world_mut()
        .spawn(Morale {
            value: 0.10,
            ..Default::default()
        })
        .id();

    app.update();

    let contagion = app
        .world()
        .get::<EmotionalContagion>(pop)
        .expect("Should add EmotionalContagion");
    assert_eq!(contagion.contagion_type, ContagionType::Terror);
}

#[test]
fn test_emotional_contagion_triggers_joy() {
    let mut app = App::new();
    app.add_plugins(bevy::MinimalPlugins);
    app.add_systems(bevy::app::Update, trigger_emotional_contagion_system);

    let pop = app
        .world_mut()
        .spawn(Morale {
            value: 0.90,
            ..Default::default()
        })
        .id();

    app.update();

    let contagion = app
        .world()
        .get::<EmotionalContagion>(pop)
        .expect("Should add EmotionalContagion");
    assert_eq!(contagion.contagion_type, ContagionType::Joy);
}

#[test]
fn test_emotional_contagion_removed_when_recovered() {
    let mut app = App::new();
    app.add_plugins(bevy::MinimalPlugins);
    app.add_systems(bevy::app::Update, trigger_emotional_contagion_system);

    let pop = app
        .world_mut()
        .spawn((
            Morale {
                value: 0.50,
                ..Default::default()
            },
            EmotionalContagion {
                contagion_type: ContagionType::Terror,
                radius: 5,
                strength: -0.15,
            },
        ))
        .id();

    app.update();

    assert!(
        app.world().get::<EmotionalContagion>(pop).is_none(),
        "Should remove EmotionalContagion when morale is normal"
    );
}
