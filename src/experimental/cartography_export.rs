use crate::layer1::beauty::BeautyGrid;
use crate::layer1::terrain::{TerrainGrid, TerrainType};
use bevy_ecs::prelude::*;
use image::{ImageBuffer, Rgb};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportPath(String);

impl ExportPath {
    pub fn new(path: &str) -> Result<Self, String> {
        if path.contains('/') || path.contains('\\') || path.contains("..") {
            return Err(
                "Invalid characters in export path. Path traversal is not allowed.".to_string(),
            );
        }
        if !path
            .chars()
            .all(|c| c.is_alphanumeric() || c == '.' || c == '-' || c == '_')
        {
            return Err("Invalid characters in export path. Only alphanumeric characters, dots, dashes, and underscores are allowed.".to_string());
        }
        Ok(Self(path.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Resource)]
pub struct CartographyExportConfig {
    pub export_path: ExportPath,
    pub trigger_export: bool,
}

impl Default for CartographyExportConfig {
    fn default() -> Self {
        Self {
            export_path: ExportPath::new("colony_map.png").expect("Default path is valid"),
            trigger_export: false,
        }
    }
}

pub fn map_export_system(
    config_opt: Option<ResMut<CartographyExportConfig>>,
    terrain: Res<TerrainGrid>,
    beauty_opt: Option<Res<BeautyGrid>>,
) {
    let mut config = match config_opt {
        Some(c) => c,
        None => return,
    };
    if !config.trigger_export {
        return;
    }

    let width = terrain.width as u32;
    let height = terrain.height as u32;
    let mut img = ImageBuffer::new(width, height);

    for y in 0..height {
        for x in 0..width {
            let tile = terrain
                .get(x as usize, y as usize)
                .unwrap_or(TerrainType::Grass);
            let mut color: [u8; 3] = match tile {
                TerrainType::Grass => [34, 139, 34],           // Forest Green
                TerrainType::Dirt => [139, 69, 19],            // Saddle Brown
                TerrainType::Rock => [128, 128, 128],          // Gray
                TerrainType::Water => [30, 144, 255],          // Dodger Blue
                TerrainType::Tree => [0, 100, 0],              // Dark Green
                TerrainType::Path => [210, 180, 140],          // Tan
                TerrainType::Shrub => [154, 205, 50],          // Yellow Green
                TerrainType::Sapling => [107, 142, 35],        // Olive Drab
                TerrainType::DeepRock => [105, 105, 105],      // Dim Gray
                TerrainType::MagmaRock => [178, 34, 34],       // Firebrick
                TerrainType::SporeBloom => [148, 0, 211],      // Dark Violet
                TerrainType::Ash => [128, 128, 128],            // Grey
                TerrainType::Artifact => [255, 215, 0],        // Gold
                TerrainType::Crater => [80, 80, 80],           // Dark Gray
                TerrainType::FaultLine(true) => [255, 60, 20], // Magma Red
                TerrainType::FaultLine(false) => [100, 100, 110],
                TerrainType::IndestructibleStump => [100, 100, 100], // Gray
                TerrainType::Bridge => [150, 100, 50],
                TerrainType::Void => [0, 0, 0],
            };

            // Optionally blend with beauty
            if let Some(ref beauty) = beauty_opt {
                let b_val = beauty.get(x as usize, y as usize);
                if b_val > 5.0 {
                    color[0] = color[0].saturating_add(30);
                    color[1] = color[1].saturating_add(30);
                    color[2] = color[2].saturating_add(30);
                } else if b_val < -5.0 {
                    color[0] = color[0].saturating_sub(30);
                    color[1] = color[1].saturating_sub(30);
                    color[2] = color[2].saturating_sub(30);
                }
            }

            img.put_pixel(x, y, Rgb(color));
        }
    }

    if let Err(e) = img.save(config.export_path.as_str()) {
        log::error!(
            "Failed to export map to {}: {}",
            config.export_path.as_str(),
            e
        );
    } else {
        log::info!(
            "Successfully exported map to {}",
            config.export_path.as_str()
        );
    }

    config.trigger_export = false;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_export_path_validation() {
        assert!(ExportPath::new("valid_map.png").is_ok());
        assert!(ExportPath::new("map-2023.png").is_ok());

        assert!(ExportPath::new("../map.png").is_err());
        assert!(ExportPath::new("dir/map.png").is_err());
        assert!(ExportPath::new("C:\\map.png").is_err());
        assert!(ExportPath::new("map*.png").is_err());
    }

    #[test]
    fn test_map_export_system() {
        let mut world = World::new();
        world.insert_resource(CartographyExportConfig {
            export_path: ExportPath::new("test_export.png").unwrap(),
            trigger_export: true,
        });

        world.insert_resource(TerrainGrid {
            width: 10,
            height: 10,
            tiles: vec![TerrainType::Grass; 100],
        });

        let mut schedule = bevy_ecs::schedule::Schedule::default();
        schedule.add_systems(map_export_system);
        schedule.run(&mut world);

        let config = world.resource::<CartographyExportConfig>();
        assert!(!config.trigger_export); // Should be reset after run
        assert!(std::path::Path::new("test_export.png").exists());

        // Clean up
        std::fs::remove_file("test_export.png").unwrap_or_default();
    }
}
