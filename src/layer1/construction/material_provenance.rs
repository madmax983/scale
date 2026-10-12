//! Spec 336 "Material Provenance": buildings inherit physical properties from
//! the raw materials used to build them. A wall raised from timber burns
//! differently than one cut from stone — the provenance is stamped at
//! construction and follows the building for its whole life.
//!
//! Two provenance paths:
//! - **Multi-material** ([`ConstructedFrom`]): the building averages the
//!   [`MaterialProperties`] of its constituent material entities (the spec's
//!   composition mechanic). Missing/despawned material entities are skipped.
//! - **Single-material** ([`Material`] component): stamps the canonical
//!   profile for the building's [`MaterialType`] — the live-game path, since
//!   construction currently selects one material per building.
//!
//! Stamping runs once per building (guarded by
//! `Without<InheritedMaterialProperties>`). A follow-on system tunes existing
//! [`Flammable`] components from the inherited flammability so material choice
//! feeds the live fire-spread sim; buildings without `Flammable` (e.g. stone)
//! are untouched.

use bevy_ecs::prelude::*;

use crate::layer1::architecture::building::{Building, Material, MaterialType};
use crate::layer1::nature::fire::Flammable;

/// Physical property profile of a raw material: how readily it burns (0.0–1.0),
/// how well it insulates against temperature swings (0.0–1.0), and how
/// beautiful it is (0.0–1.0).
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct MaterialProperties {
    /// 0.0 = incombustible, 1.0 = tinder.
    pub flammability: f32,
    /// 0.0 = thermally transparent, 1.0 = perfect insulator.
    pub insulation: f32,
    /// 0.0 = utilitarian, 1.0 = breathtaking.
    pub beauty: f32,
}

impl MaterialProperties {
    /// Canonical property profile for a construction material, consistent with
    /// the existing `MaterialType` behavior (wood burns, stone and gold are
    /// beautiful, metal is middling).
    #[must_use]
    pub const fn for_material(material: MaterialType) -> Self {
        match material {
            MaterialType::Wood => Self {
                flammability: 0.9,
                insulation: 0.6,
                beauty: 0.1,
            },
            MaterialType::Stone => Self {
                flammability: 0.0,
                insulation: 0.85,
                beauty: 0.35,
            },
            MaterialType::Metal => Self {
                flammability: 0.15,
                insulation: 0.35,
                beauty: 0.15,
            },
            MaterialType::Gold => Self {
                flammability: 0.0,
                insulation: 0.5,
                beauty: 1.0,
            },
        }
    }

    /// Arithmetic mean of a set of profiles. Returns `None` for an empty set.
    #[must_use]
    pub fn average(props: &[MaterialProperties]) -> Option<Self> {
        if props.is_empty() {
            return None;
        }
        #[allow(clippy::cast_precision_loss)]
        let n = props.len() as f32;
        Some(Self {
            flammability: props.iter().map(|p| p.flammability).sum::<f32>() / n,
            insulation: props.iter().map(|p| p.insulation).sum::<f32>() / n,
            beauty: props.iter().map(|p| p.beauty).sum::<f32>() / n,
        })
    }
}

/// Records the material entities a building was constructed from
/// (multi-material provenance — the spec's composition mechanic).
#[derive(Component, Debug, Clone, Default)]
pub struct ConstructedFrom {
    /// Material entities consumed by this building's construction.
    pub materials: Vec<Entity>,
}

/// Properties a building inherited from its construction materials,
/// stamped once at construction.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct InheritedMaterialProperties {
    /// 0.0 = incombustible, 1.0 = tinder.
    pub flammability: f32,
    /// 0.0 = thermally transparent, 1.0 = perfect insulator.
    pub insulation: f32,
    /// 0.0 = utilitarian, 1.0 = breathtaking.
    pub beauty: f32,
}

impl From<MaterialProperties> for InheritedMaterialProperties {
    fn from(props: MaterialProperties) -> Self {
        Self {
            flammability: props.flammability,
            insulation: props.insulation,
            beauty: props.beauty,
        }
    }
}

/// Stamp buildings that declare their constituent material entities:
/// average the materials' properties into [`InheritedMaterialProperties`].
/// Unresolvable material entities are skipped; buildings whose materials all
/// fail to resolve are left unstamped for a later tick.
pub fn apply_material_provenance_system(
    mut commands: Commands,
    buildings: Query<(Entity, &ConstructedFrom), Without<InheritedMaterialProperties>>,
    materials: Query<&MaterialProperties>,
) {
    for (entity, constructed_from) in buildings.iter() {
        let mut props = Vec::with_capacity(constructed_from.materials.len());
        for &mat_entity in &constructed_from.materials {
            if let Ok(p) = materials.get(mat_entity) {
                props.push(*p);
            }
        }
        if let Some(avg) = MaterialProperties::average(&props) {
            commands
                .entity(entity)
                .insert(InheritedMaterialProperties::from(avg));
        }
    }
}

/// Buildings awaiting their single-material provenance stamp.
type UnstampedSingleMaterial<'w, 's> = Query<
    'w,
    's,
    (Entity, &'static Building, &'static Material),
    (Without<InheritedMaterialProperties>, Without<ConstructedFrom>),
>;

/// Stamp single-material buildings (the live-game construction path) with the
/// canonical profile of their [`MaterialType`]. Buildings carrying
/// [`ConstructedFrom`] are skipped — the multi-material average takes
/// precedence.
pub fn stamp_single_material_provenance_system(
    mut commands: Commands,
    buildings: UnstampedSingleMaterial<'_, '_>,
) {
    for (entity, _building, material) in buildings.iter() {
        commands.entity(entity).insert(InheritedMaterialProperties::from(
            MaterialProperties::for_material(material.0),
        ));
    }
}

/// Tune existing [`Flammable`] components from freshly stamped provenance:
/// highly flammable materials get more fuel and less fire resistance.
/// Buildings without `Flammable` (e.g. stone) are untouched.
pub fn tune_flammability_system(
    stamped: Query<(Entity, &InheritedMaterialProperties), Added<InheritedMaterialProperties>>,
    mut flammables: Query<&mut Flammable>,
) {
    for (entity, inherited) in stamped.iter() {
        if let Ok(mut flammable) = flammables.get_mut(entity) {
            flammable.fire_resistance = (1.0 - inherited.flammability) * 0.5;
            flammable.fuel = 50.0 + inherited.flammability * 150.0;
        }
    }
}

/// Count buildings carrying an [`InheritedMaterialProperties`] stamp,
/// for headless visibility.
#[must_use]
pub fn stamped_building_count(world: &mut World) -> usize {
    world
        .query::<&InheritedMaterialProperties>()
        .iter(world)
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::architecture::building::BuildingType;
    use bevy_app::{App, Update};

    #[test]
    fn test_building_inherits_material_properties() {
        let mut app = App::new();
        app.add_systems(Update, apply_material_provenance_system);

        let material_entity = app
            .world_mut()
            .spawn(MaterialProperties {
                flammability: 0.8,
                insulation: 0.2,
                beauty: 0.1,
            })
            .id();

        let building_entity = app
            .world_mut()
            .spawn(ConstructedFrom {
                materials: vec![material_entity],
            })
            .id();

        app.update();

        let inherited_props = app
            .world()
            .get::<InheritedMaterialProperties>(building_entity)
            .unwrap();
        assert_eq!(inherited_props.flammability, 0.8);
        assert_eq!(inherited_props.insulation, 0.2);
        assert_eq!(inherited_props.beauty, 0.1);
    }

    #[test]
    fn test_building_averages_multiple_materials() {
        let mut app = App::new();
        app.add_systems(Update, apply_material_provenance_system);

        let material1 = app
            .world_mut()
            .spawn(MaterialProperties {
                flammability: 1.0,
                insulation: 0.0,
                beauty: 0.0,
            })
            .id();

        let material2 = app
            .world_mut()
            .spawn(MaterialProperties {
                flammability: 0.0,
                insulation: 1.0,
                beauty: 1.0,
            })
            .id();

        let building_entity = app
            .world_mut()
            .spawn(ConstructedFrom {
                materials: vec![material1, material2],
            })
            .id();

        app.update();

        let inherited_props = app
            .world()
            .get::<InheritedMaterialProperties>(building_entity)
            .unwrap();
        assert_eq!(inherited_props.flammability, 0.5);
        assert_eq!(inherited_props.insulation, 0.5);
        assert_eq!(inherited_props.beauty, 0.5);
    }

    #[test]
    fn test_single_material_building_stamped_from_material_component() {
        // Live-game path — one MaterialType, no ConstructedFrom.
        let mut app = App::new();
        app.add_systems(Update, stamp_single_material_provenance_system);

        let building_entity = app
            .world_mut()
            .spawn((
                Building {
                    building_type: BuildingType::Wall,
                },
                Material(MaterialType::Wood),
            ))
            .id();

        app.update();

        let inherited = app
            .world()
            .get::<InheritedMaterialProperties>(building_entity)
            .unwrap();
        let expected = MaterialProperties::for_material(MaterialType::Wood);
        assert_eq!(inherited.flammability, expected.flammability);
        assert_eq!(inherited.insulation, expected.insulation);
        assert_eq!(inherited.beauty, expected.beauty);
    }

    #[test]
    fn test_missing_material_entities_are_skipped() {
        let mut app = App::new();
        app.add_systems(Update, apply_material_provenance_system);

        let doomed = app.world_mut().spawn_empty().id();
        app.world_mut().despawn(doomed);
        let live = app
            .world_mut()
            .spawn(MaterialProperties {
                flammability: 0.8,
                insulation: 0.2,
                beauty: 0.1,
            })
            .id();

        let building_entity = app
            .world_mut()
            .spawn(ConstructedFrom {
                materials: vec![doomed, live],
            })
            .id();

        app.update();

        // Averaged over the resolvable material only.
        let inherited = app
            .world()
            .get::<InheritedMaterialProperties>(building_entity)
            .unwrap();
        assert_eq!(inherited.flammability, 0.8);
        assert_eq!(inherited.insulation, 0.2);
        assert_eq!(inherited.beauty, 0.1);
    }

    #[test]
    fn test_unresolvable_materials_leave_building_unstamped() {
        let mut app = App::new();
        app.add_systems(Update, apply_material_provenance_system);

        let doomed = app.world_mut().spawn_empty().id();
        app.world_mut().despawn(doomed);

        let building_entity = app
            .world_mut()
            .spawn(ConstructedFrom {
                materials: vec![doomed],
            })
            .id();

        app.update();

        assert!(app
            .world()
            .get::<InheritedMaterialProperties>(building_entity)
            .is_none());
    }

    #[test]
    fn test_constructed_from_takes_precedence_over_single_material() {
        let mut app = App::new();
        app.add_systems(
            Update,
            (
                apply_material_provenance_system,
                stamp_single_material_provenance_system,
            )
                .chain(),
        );

        let material_entity = app
            .world_mut()
            .spawn(MaterialProperties {
                flammability: 0.8,
                insulation: 0.2,
                beauty: 0.1,
            })
            .id();

        let building_entity = app
            .world_mut()
            .spawn((
                Building {
                    building_type: BuildingType::Wall,
                },
                Material(MaterialType::Stone),
                ConstructedFrom {
                    materials: vec![material_entity],
                },
            ))
            .id();

        app.update();

        // Wood average, not the stone canonical profile.
        let inherited = app
            .world()
            .get::<InheritedMaterialProperties>(building_entity)
            .unwrap();
        assert_eq!(inherited.flammability, 0.8);
    }

    #[test]
    fn test_provenance_stamps_exactly_once() {
        let mut app = App::new();
        app.add_systems(Update, apply_material_provenance_system);

        let material_entity = app
            .world_mut()
            .spawn(MaterialProperties {
                flammability: 0.8,
                insulation: 0.2,
                beauty: 0.1,
            })
            .id();

        let building_entity = app
            .world_mut()
            .spawn((
                ConstructedFrom {
                    materials: vec![material_entity],
                },
                InheritedMaterialProperties {
                    flammability: 0.1,
                    insulation: 0.1,
                    beauty: 0.1,
                },
            ))
            .id();

        app.update();

        let inherited = app
            .world()
            .get::<InheritedMaterialProperties>(building_entity)
            .unwrap();
        assert_eq!(inherited.flammability, 0.1);
    }

    #[test]
    fn test_flammability_tuning_applies_to_wood_not_stone() {
        let mut app = App::new();
        app.add_systems(
            Update,
            (
                stamp_single_material_provenance_system,
                tune_flammability_system,
            )
                .chain(),
        );

        let wood_wall = app
            .world_mut()
            .spawn((
                Building {
                    building_type: BuildingType::Wall,
                },
                Material(MaterialType::Wood),
                Flammable::default(),
            ))
            .id();
        let stone_wall = app
            .world_mut()
            .spawn((
                Building {
                    building_type: BuildingType::Wall,
                },
                Material(MaterialType::Stone),
            ))
            .id();

        app.update();

        let wood_flammable = app.world().get::<Flammable>(wood_wall).unwrap();
        assert!(wood_flammable.fuel > 100.0);
        assert!(wood_flammable.fire_resistance < 0.5);
        assert!(app.world().get::<Flammable>(stone_wall).is_none());

        let stone_inherited = app
            .world()
            .get::<InheritedMaterialProperties>(stone_wall)
            .unwrap();
        assert_eq!(stone_inherited.flammability, 0.0);
    }

    #[test]
    fn test_canonical_profiles_cover_all_materials() {
        for material in [
            MaterialType::Wood,
            MaterialType::Stone,
            MaterialType::Metal,
            MaterialType::Gold,
        ] {
            let props = MaterialProperties::for_material(material);
            assert!((0.0..=1.0).contains(&props.flammability));
            assert!((0.0..=1.0).contains(&props.insulation));
            assert!((0.0..=1.0).contains(&props.beauty));
        }
        assert!(MaterialProperties::for_material(MaterialType::Wood).flammability > 0.5);
        assert_eq!(
            MaterialProperties::for_material(MaterialType::Stone).flammability,
            0.0
        );
        assert_eq!(
            MaterialProperties::for_material(MaterialType::Gold).beauty,
            1.0
        );
    }

    #[test]
    fn test_average_empty_returns_none() {
        assert!(MaterialProperties::average(&[]).is_none());
    }
}
