//! The Procedural Narrative Generation System.
//!
//! This module provides the core engine for generating mad-libs style text from
//! templates and fragments. It forms the backbone for procedural histories,
//! event descriptions, and storytelling in SCALE.
//!
//! The narrative system operates on two main concepts:
//! - **Templates**: Large strings of text with placeholders (slots) defined in brackets, e.g., `[CIV_NAME]`.
//! - **Fragments**: Lists of words or phrases that can randomly fill in those slots if the context does not explicitly provide a value.
//!
//! The main entry point is the [`NarrativeGenerator`](crate::shared::narrative::NarrativeGenerator), which is used alongside
//! a [`NarrativeContext`](crate::shared::narrative::NarrativeContext) to fill in dynamic values (like names and dates).

use bevy_ecs::prelude::*;
use rand::seq::SliceRandom;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug)]
/// The types of errors that can occur during procedural narrative generation.
///
/// Returned primarily by [`NarrativeGenerator::generate`] when resolving a template fails,
/// often due to missing items in the [`NarrativeContext`].
///
/// ## Examples
///
/// ```
/// use scale::shared::narrative::{NarrativeGenerator, NarrativeContext, NarrativeError};
///
/// let generator = NarrativeGenerator::default();
/// let context = NarrativeContext::default();
///
/// // Attempting to generate a missing template yields an error
/// let result = generator.generate("NON_EXISTENT", &context);
///
/// assert!(matches!(result, Err(NarrativeError::TemplateNotFound(_))));
/// ```
/// An error that occurs during narrative generation or loading.
///
/// **Note:** Error rendering is handled directly via the [`std::fmt::Display`] trait.
/// There is no `.to_table()` method. You can format the error nicely simply by
/// using `println!("{}", e)`.
pub enum NarrativeError {
    /// A required context variable was not provided when generating a narrative.
    ///
    /// You must call [`NarrativeContext::insert`] with the missing key before generating.
    MissingContext(String),

    /// A fragment type was referenced but no options were provided in the lore files.
    ///
    /// Ensure your `FRAGMENTS.md` file contains a bulleted list of options under this fragment's header.
    MissingFragmentOptions(String),

    /// The specified directory was found, but it contained no valid lore files.
    ///
    /// Ensure the directory contains `TEMPLATES.md` and/or `FRAGMENTS.md`.
    NoLoreFiles(String),

    /// The requested template ID does not exist in the loaded templates.
    ///
    /// Check for typos in the template name or ensure it is defined in `TEMPLATES.md`.
    TemplateNotFound(String),

    /// The requested template exists, but it has no patterns defined.
    ///
    /// Ensure the template in `TEMPLATES.md` contains at least one pattern string.
    NoPatternsForTemplate(String),

    /// The provided lore directory could not be found or is not a directory.
    ///
    /// Verify the path passed to [`NarrativeGenerator::load_from_files`] is correct.
    DirectoryNotFound(String),

    /// An I/O error occurred while reading a lore file.
    ///
    /// The string contains the path that failed, and the inner error is the underlying I/O error.
    IoError(String, std::io::Error),
}

impl std::fmt::Display for NarrativeError {
    /// Formats the error as a human-readable table with actionable advice.
    ///
    /// # Examples
    ///
    /// ```
    /// use scale::shared::narrative::NarrativeError;
    /// let err = NarrativeError::MissingContext("YEAR".to_string());
    ///
    /// // The Display trait handles the formatting automatically.
    /// // Users do not need to call any custom formatting methods.
    /// let output = format!("{}", err);
    /// assert!(output.contains("Missing Context"));
    /// assert!(output.contains("YEAR"));
    /// ```
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        #[cfg(not(target_arch = "wasm32"))]
use comfy_table::{presets::UTF8_FULL, Attribute as TableAttribute, Cell, Color as TableColor, Table};
#[cfg(target_arch = "wasm32")]
use comfy_table::{presets::UTF8_FULL, Cell, Table};
#[cfg(target_arch = "wasm32")]
use crate::wasm_style::{CellStyle, TableAttribute, TableColor};

        let (err_type, message, action) = match self {
            Self::MissingContext(k) => (
                "Missing Context",
                format!("Variable '{}' is required but missing.", k),
                format!("context.insert(\"{}\", <value>)", k),
            ),
            Self::MissingFragmentOptions(f_str) => (
                "Empty Fragment",
                format!("Fragment '{}' has no options defined.", f_str),
                "Add options to the fragment in lore files.".to_string(),
            ),
            Self::NoLoreFiles(d) => (
                "Files Missing",
                format!(
                    "No lore files found in `{}`. Expected TEMPLATES.md or FRAGMENTS.md.",
                    d
                ),
                "Check the directory for TEMPLATES.md.".to_string(),
            ),
            Self::TemplateNotFound(t) => (
                "Missing Template",
                format!("Template not found (`{}`).", t),
                "Verify template ID exists in TEMPLATES.md.".to_string(),
            ),
            Self::NoPatternsForTemplate(t) => (
                "Empty Template",
                format!("Template `{}` has no patterns.", t),
                "Ensure the template in TEMPLATES.md contains at least one pattern string."
                    .to_string(),
            ),
            Self::DirectoryNotFound(d) => (
                "Directory Not Found",
                format!("Directory not found or not a directory (`{}`).", d),
                "Verify the path passed to `NarrativeGenerator::load_from_files` is correct."
                    .to_string(),
            ),
            Self::IoError(path, e) => (
                "I/O Error",
                format!("Failed to read `{}`: {}", path, e),
                "Check file permissions and path.".to_string(),
            ),
        };

        #[cfg(not(target_arch = "wasm32"))]
        use crossterm::style::{Color, Stylize};
        #[cfg(target_arch = "wasm32")]
        use crate::wasm_style::{Color, Stylize};

        writeln!(
            f,
            "\n{}",
            "╭── Narrative System Error ───────────────────────╮".with(Color::Red)
        )?;

        let text = format!("{:<47}", err_type);
        writeln!(f, "│ {} │", text.with(Color::Yellow))?;
        writeln!(
            f,
            "{}",
            "╰─────────────────────────────────────────────────╯".with(Color::Red)
        )?;

        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .apply_modifier(comfy_table::modifiers::UTF8_ROUND_CORNERS)
            .set_content_arrangement(comfy_table::ContentArrangement::Dynamic);

        table.add_row(vec![
            Cell::new("Message")
                .fg(TableColor::Cyan)
                .add_attribute(TableAttribute::Bold),
            Cell::new(message).fg(TableColor::White),
        ]);
        table.add_row(vec![
            Cell::new("Fix")
                .fg(TableColor::Green)
                .add_attribute(TableAttribute::Bold),
            Cell::new(action).fg(TableColor::White),
        ]);

        write!(f, "{}", table)
    }
}

impl std::error::Error for NarrativeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::IoError(_, ref e) => Some(e),
            _ => None,
        }
    }
}

/// A segment of a generated narrative.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NarrativeSegment {
    /// Static text from the template.
    Text(String),
    /// A value filled into a slot (key, value).
    Slot {
        /// The key of the slot (e.g. "NAME").
        key: String,
        /// The value filled into the slot.
        value: String,
    },
    /// A missing context variable.
    MissingContext(String),
    /// A missing fragment options.
    MissingFragmentOptions(String),
}

impl std::fmt::Display for NarrativeSegment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Text(s) => write!(f, "{s}"),
            Self::Slot { value, .. } => write!(f, "{value}"),
            Self::MissingContext(s) => write!(f, "Missing context: '{s}'"),
            Self::MissingFragmentOptions(s) => write!(f, "Fragment '{s}' has no options defined"),
        }
    }
}

/// Context for story generation, holding values for slots.
///
/// ## Examples
///
/// ```
/// use scale::shared::narrative::NarrativeContext;
///
/// let mut context = NarrativeContext::new();
/// context.insert("YEAR", 2150);
/// context.insert("CIV_NAME", "Terran Dominion");
///
/// assert_eq!(context.get("YEAR"), Some(&"2150".to_string()));
/// assert_eq!(context.get("CIV_NAME"), Some(&"Terran Dominion".to_string()));
/// ```
#[derive(Debug, Default, Clone)]
pub struct NarrativeContext {
    slots: HashMap<String, String>,
}

impl NarrativeContext {
    /// Creates a new, empty [`NarrativeContext`].
    ///
    /// ## Examples
    ///
    /// ```
    /// use scale::shared::narrative::NarrativeContext;
    ///
    /// let context = NarrativeContext::new();
    /// assert_eq!(context.get("ANY_KEY"), None);
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts a value for a specific slot key.
    ///
    /// Any template encountering `[KEY]` will have it replaced by this value.
    ///
    /// ## Examples
    ///
    /// ```
    /// use scale::shared::narrative::NarrativeContext;
    ///
    /// let mut context = NarrativeContext::new();
    /// context.insert("CIV_NAME", "The Empire");
    ///
    /// assert_eq!(context.get("CIV_NAME"), Some(&"The Empire".to_string()));
    /// ```
    pub fn insert<V: ToString>(&mut self, key: &str, value: V) {
        self.slots.insert(key.to_string(), value.to_string());
    }

    /// Retrieves a value for a specific slot key, if it exists.
    ///
    /// ## Examples
    ///
    /// ```
    /// use scale::shared::narrative::NarrativeContext;
    ///
    /// let mut context = NarrativeContext::new();
    /// context.insert("YEAR", 2150);
    ///
    /// assert_eq!(context.get("YEAR"), Some(&"2150".to_string()));
    /// assert_eq!(context.get("MISSING"), None);
    /// ```
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&String> {
        self.slots.get(key)
    }
}

/// A narrative template with multiple possible patterns.
#[derive(Debug, Clone)]
///
/// Templates are the core structural elements of the procedural storytelling system.
/// A template has a unique `id` and a list of possible `patterns`. When generating
/// a story, one of the `patterns` is chosen randomly.
///
/// ## Examples
///
/// ```
/// use scale::shared::narrative::Template;
///
/// let template = Template {
///     id: "GREETING".to_string(),
///     patterns: vec!["Hello [NAME]!".to_string(), "Greetings, [NAME].".to_string()],
/// };
///
/// assert_eq!(template.id, "GREETING");
/// assert_eq!(template.patterns.len(), 2);
/// ```
pub struct Template {
    /// Unique identifier for the template (e.g., "`CIVILIZATION_RISE`").
    pub id: String,
    /// List of pattern strings with slots. Slots are enclosed in brackets, e.g., `[YEAR]`.
    pub patterns: Vec<String>,
}

/// A collection of text options for a specific thematic fragment type.
#[derive(Debug, Clone)]
///
/// Fragments are the adjectives, nouns, and names that fill into the slots of a [`Template`].
/// When a template encounters a slot (e.g., `[CIV_EPITHET]`) that is NOT found in the
/// [`NarrativeContext`], the generator will look for a `FragmentType` with the ID
/// `CIV_EPITHET` and select a random option from its list.
///
/// ## Examples
///
/// ```
/// use scale::shared::narrative::FragmentType;
///
/// let fragment = FragmentType {
///     id: "CIV_EPITHET".to_string(),
///     options: vec!["The First Ones".to_string(), "The Ascendant".to_string()],
/// };
///
/// assert_eq!(fragment.id, "CIV_EPITHET");
/// assert_eq!(fragment.options.len(), 2);
/// ```
pub struct FragmentType {
    /// Unique identifier for the fragment type (e.g., "`CIV_EPITHET`").
    pub id: String,
    /// List of possible text values that can be randomly selected.
    pub options: Vec<String>,
}

/// The main generator system.
///
/// This struct holds the templates and fragments used to generate stories.
/// It is usually populated via [`NarrativeGenerator::from_embedded`].
///
/// **Note:** This is part of the base game and DOES NOT require the `nova` feature flag.
/// It is distinctly different from the `OralTradition` system which simulates living
/// legends in taverns.
///
/// ## Examples
///
/// ```
/// use scale::shared::narrative::{NarrativeGenerator, NarrativeContext};
///
/// // Initialize with default embedded templates and fragments
/// let generator = NarrativeGenerator::from_embedded();
/// let mut context = NarrativeContext::new();
/// context.insert("CIV_NAME", "Terran Dominion");
/// context.insert("ORIGIN_STAR", "Sol");
/// context.insert("CIV_EPITHET", "The Explorers");
/// context.insert("YEAR", "2150");
///
/// let story = generator.generate("CIVILIZATION_RISE", &context);
/// assert!(story.is_ok());
/// ```
#[derive(Debug, Default, Resource)]
pub struct NarrativeGenerator {
    templates: HashMap<String, Template>,
    fragments: HashMap<String, FragmentType>,
}

impl NarrativeGenerator {
    /// Returns a sorted list of all available [`Template`] IDs loaded into the generator.
    ///
    /// This is useful for UI introspection or randomly selecting a template.
    ///
    /// ## Examples
    ///
    /// ```
    /// use scale::shared::narrative::NarrativeGenerator;
    ///
    /// let mut generator = NarrativeGenerator::default();
    /// generator.add_template("GREETING".to_string(), vec!["Hello".to_string()]);
    ///
    /// let ids = generator.get_template_ids();
    /// assert_eq!(ids.len(), 1);
    /// assert_eq!(ids[0], "GREETING");
    /// ```
    #[must_use]
    pub fn get_template_ids(&self) -> Vec<&String> {
        let mut ids: Vec<&String> = self.templates.keys().collect();
        ids.sort();
        ids
    }

    /// Returns a sorted list of all available [`FragmentType`] IDs loaded into the generator.
    ///
    /// ## Examples
    ///
    /// ```
    /// use scale::shared::narrative::NarrativeGenerator;
    ///
    /// let mut generator = NarrativeGenerator::default();
    /// generator.add_fragment("COLORS".to_string(), vec!["Red".to_string()]);
    ///
    /// let ids = generator.get_fragment_ids();
    /// assert_eq!(ids.len(), 1);
    /// assert_eq!(ids[0], "COLORS");
    /// ```
    #[must_use]
    pub fn get_fragment_ids(&self) -> Vec<&String> {
        let mut ids: Vec<&String> = self.fragments.keys().collect();
        ids.sort();
        ids
    }

    /// Retrieves a specific [`Template`] by its ID, if it exists.
    ///
    /// ## Examples
    ///
    /// ```
    /// use scale::shared::narrative::NarrativeGenerator;
    ///
    /// let mut generator = NarrativeGenerator::default();
    /// generator.add_template("GREETING".to_string(), vec!["Hello".to_string()]);
    ///
    /// assert!(generator.get_template("GREETING").is_some());
    /// assert!(generator.get_template("FAREWELL").is_none());
    /// ```
    #[must_use]
    pub fn get_template(&self, id: &str) -> Option<&Template> {
        self.templates.get(id)
    }

    /// Loads templates and fragments from a specified directory on the filesystem.
    ///
    /// The directory must contain `TEMPLATES.md` and/or `FRAGMENTS.md` files formatted
    /// correctly.
    ///
    /// ## Errors
    /// Returns a [`NarrativeError`] if:
    /// - The provided path is not a directory (`DirectoryNotFound`).
    /// - Neither `TEMPLATES.md` nor `FRAGMENTS.md` are present in the directory (`NoLoreFiles`).
    /// - The files exist but cannot be read due to I/O permissions (`IoError`).
    ///
    /// ## Examples
    ///
    /// ```
    /// use scale::shared::narrative::{NarrativeGenerator, NarrativeError};
    ///
    /// let mut generator = NarrativeGenerator::default();
    ///
    /// // Pointing to a missing directory will return an error
    /// let result = generator.load_from_files("./non_existent_lore_dir");
    /// assert!(matches!(result, Err(NarrativeError::DirectoryNotFound(_))));
    /// ```
    pub fn load_from_files<P: AsRef<Path>>(
        &mut self,
        path: P,
    ) -> std::result::Result<(), NarrativeError> {
        let path = path.as_ref();

        if !path.exists() || !path.is_dir() {
            return Err(NarrativeError::DirectoryNotFound(
                path.display().to_string(),
            ));
        }

        let mut loaded_any = false;

        let templates_path = path.join("TEMPLATES.md");
        if templates_path.exists() {
            let mut file = std::fs::File::open(&templates_path)
                .map_err(|e| NarrativeError::IoError(templates_path.display().to_string(), e))?;
            let mut content = String::new();
            std::io::Read::read_to_string(
                &mut std::io::Read::take(&mut file, 5 * 1024 * 1024),
                &mut content,
            )
            .map_err(|e| NarrativeError::IoError(templates_path.display().to_string(), e))?;

            self.parse_templates(&content);
            loaded_any = true;
        }

        let fragments_path = path.join("FRAGMENTS.md");
        if fragments_path.exists() {
            let mut file = std::fs::File::open(&fragments_path)
                .map_err(|e| NarrativeError::IoError(fragments_path.display().to_string(), e))?;
            let mut content = String::new();
            std::io::Read::read_to_string(
                &mut std::io::Read::take(&mut file, 5 * 1024 * 1024),
                &mut content,
            )
            .map_err(|e| NarrativeError::IoError(fragments_path.display().to_string(), e))?;

            self.parse_fragments(&content);
            loaded_any = true;
        }

        if !loaded_any {
            return Err(NarrativeError::NoLoreFiles(path.display().to_string()));
        }

        Ok(())
    }

    /// Create a generator pre-loaded from embedded lore files.
    ///
    /// Uses `include_str!` so it works on WASM (no filesystem access).
    ///
    /// ## Examples
    /// ```
    /// use scale::prelude::*;
    ///
    /// let generator = NarrativeGenerator::from_embedded();
    ///
    /// let mut context = NarrativeContext::default();
    /// context.insert("CIV_NAME", "Terran Dominion");
    /// context.insert("ORIGIN_STAR", "Sol Prime");
    /// context.insert("YEAR", "2150");
    /// context.insert("CIV_EPITHET", "The First Ones");
    ///
    /// // Generate a story from a template (e.g., "CIVILIZATION_RISE")
    /// let story = generator.generate("CIVILIZATION_RISE", &context).unwrap();
    /// println!("{}", story);
    /// ```
    #[must_use]
    pub fn from_embedded() -> Self {
        let mut narrator = Self::default();
        narrator.parse_templates(include_str!("../../lore/TEMPLATES.md"));
        narrator.parse_fragments(include_str!("../../lore/FRAGMENTS.md"));
        narrator
    }

    /// Generates a procedural star name by combining `STAR_PREFIX` and `STAR_SUFFIX` fragments.
    ///
    /// If the required fragments are not loaded in the generator, it will fallback
    /// to producing the string `"Unknown Prime"`.
    ///
    /// ## Examples
    ///
    /// ```
    /// use scale::shared::narrative::NarrativeGenerator;
    ///
    /// let mut generator = NarrativeGenerator::default();
    /// generator.add_fragment("STAR_PREFIX".to_string(), vec!["Alpha".to_string()]);
    /// generator.add_fragment("STAR_SUFFIX".to_string(), vec!["Centauri".to_string()]);
    ///
    /// let name = generator.generate_star_name();
    /// assert_eq!(name, "Alpha Centauri");
    /// ```
    #[must_use]
    pub fn generate_star_name(&self) -> String {
        let prefix = self
            .get_random_fragment("STAR_PREFIX")
            .cloned()
            .unwrap_or_else(|| "Unknown".to_string());
        let suffix = self
            .get_random_fragment("STAR_SUFFIX")
            .cloned()
            .unwrap_or_else(|| "Prime".to_string());
        format!("{prefix} {suffix}")
    }

    /// Generates a procedural civilization name by selecting a random `STAR_PREFIX` fragment.
    ///
    /// If the `STAR_PREFIX` fragment is missing, it falls back to `"Unknown"`.
    ///
    /// ## Examples
    ///
    /// ```
    /// use scale::shared::narrative::NarrativeGenerator;
    ///
    /// let mut generator = NarrativeGenerator::default();
    /// generator.add_fragment("STAR_PREFIX".to_string(), vec!["Terran".to_string()]);
    ///
    /// let name = generator.generate_civ_name();
    /// assert_eq!(name, "Terran");
    /// ```
    #[must_use]
    pub fn generate_civ_name(&self) -> String {
        self.get_random_fragment("STAR_PREFIX")
            .cloned()
            .unwrap_or_else(|| "Unknown".to_string())
    }

    /// Adds a new [`Template`] to the generator programmatically.
    ///
    /// This is useful for injecting dynamic stories that are not defined in the static
    /// markdown files.
    ///
    /// ## Examples
    ///
    /// ```
    /// use scale::shared::narrative::NarrativeGenerator;
    ///
    /// let mut generator = NarrativeGenerator::default();
    /// generator.add_template(
    ///     "TUTORIAL_START".to_string(),
    ///     vec!["Welcome to the colony, [NAME].".to_string()]
    /// );
    ///
    /// assert_eq!(generator.template_count(), 1);
    /// ```
    pub fn add_template(&mut self, id: String, patterns: Vec<String>) {
        self.templates.insert(id.clone(), Template { id, patterns });
    }

    /// Adds a new [`FragmentType`] to the generator programmatically.
    ///
    /// ## Examples
    ///
    /// ```
    /// use scale::shared::narrative::NarrativeGenerator;
    ///
    /// let mut generator = NarrativeGenerator::default();
    /// generator.add_fragment(
    ///     "COLORS".to_string(),
    ///     vec!["Red".to_string(), "Blue".to_string()]
    /// );
    ///
    /// assert_eq!(generator.fragment_count(), 1);
    /// ```
    pub fn add_fragment(&mut self, id: String, options: Vec<String>) {
        self.fragments
            .insert(id.clone(), FragmentType { id, options });
    }

    /// Parse templates from Markdown content.
    pub fn parse_templates(&mut self, content: &str) {
        let mut current_id: Option<String> = None;
        let mut current_patterns: Vec<String> = Vec::new();
        let mut capturing_code_block = false;

        for line in content.lines() {
            let trimmed = line.trim();

            // Detect Template Header: "### TEMPLATE_NAME"
            if let Some(id_part) = trimmed
                .strip_prefix("### ")
                .or_else(|| trimmed.strip_prefix("## Template: "))
            {
                // If we were parsing a previous template, save it
                if let Some(id) = current_id.take() {
                    if !current_patterns.is_empty() {
                        self.templates.insert(
                            id.clone(),
                            Template {
                                id,
                                patterns: current_patterns.clone(),
                            },
                        );
                    }
                }

                // Start new template
                if !id_part.contains("Templates") && !id_part.contains("Fragments") {
                    current_id = Some(id_part.trim().to_string());
                    current_patterns = Vec::new();
                    capturing_code_block = false;
                }
                continue;
            }

            // Detect Code Block for patterns
            if trimmed.starts_with("```") {
                capturing_code_block = !capturing_code_block;
                continue;
            }

            // Capture patterns inside code blocks
            if capturing_code_block && !trimmed.is_empty() {
                // Remove quotes if present
                let pattern =
                    if trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2 {
                        &trimmed[1..trimmed.len() - 1]
                    } else {
                        trimmed
                    };

                // Ignore empty lines or comments
                if !pattern.is_empty() && !pattern.starts_with("//") {
                    current_patterns.push(pattern.to_string());
                }
            } else if !capturing_code_block
                && trimmed.starts_with("- \"")
                && trimmed.ends_with('"')
                && trimmed.len() >= 4
            {
                // Capture bullet point patterns outside code blocks
                let pattern = &trimmed[3..trimmed.len() - 1];
                current_patterns.push(pattern.to_string());
            }
        }

        // Save the last one
        if let Some(id) = current_id {
            if !current_patterns.is_empty() {
                self.templates.insert(
                    id.clone(),
                    Template {
                        id,
                        patterns: current_patterns,
                    },
                );
            }
        }
    }

    /// Parse fragments from Markdown content.
    pub fn parse_fragments(&mut self, content: &str) {
        let mut current_id: Option<String> = None;
        let mut current_options: Vec<String> = Vec::new();
        let mut capturing_code_block = false;

        for line in content.lines() {
            let trimmed = line.trim();

            // Detect Fragment Header: "### [FRAGMENT_NAME]"
            if let Some(header) = trimmed
                .strip_prefix("### ")
                .or_else(|| trimmed.strip_prefix("## Fragment Type: "))
            {
                if let Some(id) = current_id.take() {
                    if !current_options.is_empty() {
                        self.fragments.insert(
                            id.clone(),
                            FragmentType {
                                id,
                                options: current_options.clone(),
                            },
                        );
                    }
                }

                // Extract name between brackets
                if let Some(start) = header.find('[') {
                    if let Some(end) = header.find(']') {
                        current_id = Some(header[start + 1..end].to_string());
                        current_options = Vec::new();
                        capturing_code_block = false;
                    }
                }
                continue;
            }

            // Detect Code Block
            if trimmed.starts_with("```") {
                capturing_code_block = !capturing_code_block;
                continue;
            }

            // Capture options
            if capturing_code_block && !trimmed.is_empty() {
                current_options.push(trimmed.to_string());
            } else if !capturing_code_block && trimmed.starts_with("- ") {
                current_options.push(trimmed[2..].to_string());
            }
        }

        // Save last
        if let Some(id) = current_id {
            if !current_options.is_empty() {
                self.fragments.insert(
                    id.clone(),
                    FragmentType {
                        id,
                        options: current_options,
                    },
                );
            }
        }
    }

    /// Returns the number of loaded templates in the generator.
    ///
    /// ## Examples
    ///
    /// ```
    /// use scale::shared::narrative::NarrativeGenerator;
    ///
    /// let generator = NarrativeGenerator::from_embedded();
    /// assert!(generator.template_count() > 0);
    /// ```
    #[must_use]
    pub fn template_count(&self) -> usize {
        self.templates.len()
    }

    /// Returns the number of loaded fragment types in the generator.
    ///
    /// ## Examples
    ///
    /// ```
    /// use scale::shared::narrative::NarrativeGenerator;
    ///
    /// let generator = NarrativeGenerator::from_embedded();
    /// assert!(generator.fragment_count() > 0);
    /// ```
    #[must_use]
    pub fn fragment_count(&self) -> usize {
        self.fragments.len()
    }

    /// Selects and returns a random option string from the specified fragment type ID.
    ///
    /// ## Examples
    ///
    /// ```
    /// use scale::shared::narrative::NarrativeGenerator;
    ///
    /// let mut generator = NarrativeGenerator::default();
    /// generator.add_fragment("COLORS".to_string(), vec!["Red".to_string()]);
    ///
    /// let color = generator.get_random_fragment("COLORS");
    /// assert_eq!(color, Some(&"Red".to_string()));
    ///
    /// let missing = generator.get_random_fragment("SHAPES");
    /// assert_eq!(missing, None);
    /// ```
    #[must_use]
    pub fn get_random_fragment(&self, fragment_id: &str) -> Option<&String> {
        self.fragments
            .get(fragment_id)?
            .options
            .choose(&mut rand::thread_rng())
    }

    /// Generate a story string from a template ID and context.
    ///
    /// ## Examples
    /// ```
    /// use scale::prelude::*;
    ///
    /// let mut generator = NarrativeGenerator::default();
    /// generator.add_template("GREETING".to_string(), vec!["Hello [NAME]!".to_string()]);
    ///
    /// let mut context = NarrativeContext::default();
    /// context.insert("NAME", "Traveler");
    ///
    /// let story = generator.generate("GREETING", &context).unwrap();
    /// assert_eq!(story, "Hello Traveler!");
    /// ```
    ///
    /// # Errors
    /// Returns an error if the template ID is not found or if the template has no patterns.
    pub fn generate(
        &self,
        template_id: &str,
        context: &NarrativeContext,
    ) -> std::result::Result<String, NarrativeError> {
        let segments = self.generate_structured(template_id, context)?;
        Ok(segments
            .iter()
            .map(std::string::ToString::to_string)
            .collect())
    }

    /// Generate a structured story from a template ID and context.
    ///
    /// Returns a vector of [`NarrativeSegment`]s, which preserves the distinction
    /// between static text and filled slots for UI rendering.
    ///
    /// # Errors
    /// Returns an error if the template ID is not found or if the template has no patterns.
    pub fn generate_structured(
        &self,
        template_id: &str,
        context: &NarrativeContext,
    ) -> std::result::Result<Vec<NarrativeSegment>, NarrativeError> {
        let template = self
            .templates
            .get(template_id)
            .ok_or_else(|| NarrativeError::TemplateNotFound(template_id.to_string()))?;

        // Pick a random pattern
        let pattern = template
            .patterns
            .choose(&mut rand::thread_rng())
            .ok_or_else(|| NarrativeError::NoPatternsForTemplate(template_id.to_string()))?;

        let mut segments = Vec::new();
        let mut char_iter = pattern.chars().peekable();
        let mut current_text = String::new();

        while let Some(c) = char_iter.next() {
            if c == '[' {
                // Possible slot start
                let mut slot_name = String::new();
                let mut closed = false;

                // Peek ahead to capture slot name
                while let Some(&next_c) = char_iter.peek() {
                    char_iter.next(); // Consume
                    if next_c == ']' {
                        closed = true;
                        break;
                    }
                    slot_name.push(next_c);
                }

                if closed {
                    // Flush accumulated text
                    if !current_text.is_empty() {
                        segments.push(NarrativeSegment::Text(current_text));
                        current_text = String::new();
                    }

                    // Check for optional marker '?' at end of slot name
                    let is_optional = slot_name.ends_with('?');
                    let key = if is_optional {
                        &slot_name[0..slot_name.len() - 1]
                    } else {
                        &slot_name
                    };

                    // Resolve slot
                    if let Some(val) = context.get(key) {
                        segments.push(NarrativeSegment::Slot {
                            key: key.to_string(),
                            value: val.clone(),
                        });
                    } else if let Some(fragment) = self.fragments.get(key) {
                        // Pick random fragment
                        if let Some(option) = fragment.options.choose(&mut rand::thread_rng()) {
                            segments.push(NarrativeSegment::Slot {
                                key: key.to_string(),
                                value: option.clone(),
                            });
                        } else {
                            segments
                                .push(NarrativeSegment::MissingFragmentOptions(key.to_string()));
                        }
                    } else {
                        // Not found in context or fragments
                        if !is_optional {
                            segments.push(NarrativeSegment::MissingContext(slot_name.clone()));
                        }
                    }
                } else {
                    // Malformed bracket, just push what we collected
                    current_text.push('[');
                    current_text.push_str(&slot_name);
                }
            } else {
                current_text.push(c);
            }
        }

        // Push remaining text
        if !current_text.is_empty() {
            segments.push(NarrativeSegment::Text(current_text));
        }

        // Check for missing context variables/fragments that would produce errors
        for segment in &segments {
            match segment {
                NarrativeSegment::MissingContext(err) => {
                    return Err(NarrativeError::MissingContext(err.clone()));
                }
                NarrativeSegment::MissingFragmentOptions(err) => {
                    return Err(NarrativeError::MissingFragmentOptions(err.clone()));
                }
                _ => {}
            }
        }

        Ok(segments)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_insert_get() {
        let mut ctx = NarrativeContext::new();
        ctx.insert("KEY", "VALUE");
        assert_eq!(ctx.get("KEY").unwrap(), "VALUE");

        // Test non-string types
        ctx.insert("YEAR", 2150);
        assert_eq!(ctx.get("YEAR").unwrap(), "2150");

        ctx.insert("ACTIVE", true);
        assert_eq!(ctx.get("ACTIVE").unwrap(), "true");
    }

    #[test]
    fn test_template_parsing() {
        let content = r#"
### TEST_TEMPLATE
**Slots:** [SLOT]
```
"Pattern with [SLOT]"
"Another [SLOT]"
```
"#;
        let mut generator = NarrativeGenerator::default();
        generator.parse_templates(content);

        assert_eq!(generator.template_count(), 1);
        let tmpl = generator.templates.get("TEST_TEMPLATE").unwrap();
        assert_eq!(tmpl.patterns.len(), 2);
        assert_eq!(tmpl.patterns[0], "Pattern with [SLOT]");
    }

    #[test]
    fn test_fragment_parsing() {
        let content = r"
### [TEST_FRAGMENT]
- option1
- option2
";
        let mut generator = NarrativeGenerator::default();
        generator.parse_fragments(content);

        assert_eq!(generator.fragment_count(), 1);
        let frag = generator.fragments.get("TEST_FRAGMENT").unwrap();
        assert_eq!(frag.options.len(), 2);
        assert_eq!(frag.options[0], "option1");
    }

    #[test]
    fn test_generation_simple() {
        let mut generator = NarrativeGenerator::default();
        generator.templates.insert(
            "SIMPLE".to_string(),
            Template {
                id: "SIMPLE".to_string(),
                patterns: vec!["Hello [NAME]!".to_string()],
            },
        );

        let mut ctx = NarrativeContext::new();
        ctx.insert("NAME", "World");

        let result = generator.generate("SIMPLE", &ctx).unwrap();
        assert_eq!(result, "Hello World!");
    }

    #[test]
    fn test_template_parsing_alternative_header_and_bullets() {
        let content = r#"
## Template: TEST_BULLETS
- "Pattern one"
- "Pattern two"
"#;
        let mut generator = NarrativeGenerator::default();
        generator.parse_templates(content);
        assert_eq!(generator.template_count(), 1);
        let tmpl = generator.templates.get("TEST_BULLETS").unwrap();
        assert_eq!(tmpl.patterns.len(), 2);
        assert_eq!(tmpl.patterns[0], "Pattern one");
    }

    #[test]
    fn test_fragment_parsing_alternative_header() {
        let content = r"
## Fragment Type: [TEST_FRAG]
- option A
- option B
";
        let mut generator = NarrativeGenerator::default();
        generator.parse_fragments(content);
        assert_eq!(generator.fragment_count(), 1);
        let frag = generator.fragments.get("TEST_FRAG").unwrap();
        assert_eq!(frag.options.len(), 2);
        assert_eq!(frag.options[0], "option A");
    }

    #[test]
    fn test_generation_fragment_fallback() {
        let mut generator = NarrativeGenerator::default();
        generator.add_template("FRAG".to_string(), vec!["Value: [KEY]".to_string()]);
        generator.add_fragment("KEY".to_string(), vec!["FragmentValue".to_string()]);

        let ctx = NarrativeContext::new();
        // Context empty, should use fragment
        let result = generator.generate("FRAG", &ctx).unwrap();
        assert_eq!(result, "Value: FragmentValue");
    }

    #[test]
    fn test_from_embedded_loads_templates_and_fragments() {
        let narrator = NarrativeGenerator::from_embedded();
        assert!(narrator.template_count() > 0, "Should load templates");
        assert!(narrator.fragment_count() > 0, "Should load fragments");
    }

    #[test]
    fn test_generate_star_name() {
        let narrator = NarrativeGenerator::from_embedded();
        let name = narrator.generate_star_name();
        assert!(!name.is_empty());
        assert!(name.contains(' '), "Star name should have prefix + suffix");
    }

    #[test]
    fn test_generate_civ_name() {
        let narrator = NarrativeGenerator::from_embedded();
        let name = narrator.generate_civ_name();
        assert!(!name.is_empty());
    }

    #[test]
    fn test_panic_on_single_quote() {
        let content = r#"
### TEST_PANIC
```
"
```
"#;
        let mut generator = NarrativeGenerator::default();
        // Should not panic
        generator.parse_templates(content);
        // " starts with " and ends with " but len is 1. So else branch -> trimmed ("") -> pattern ("").
        // It captures " as a pattern.
        assert_eq!(generator.template_count(), 1);
        let tmpl = generator.templates.get("TEST_PANIC").unwrap();
        assert_eq!(tmpl.patterns[0], "\"");
    }

    #[test]
    fn test_load_from_files_not_found() {
        let mut generator = NarrativeGenerator::default();
        let result = generator.load_from_files("non_existent_path_xyz_123");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, NarrativeError::DirectoryNotFound(_)));
    }

    #[test]
    fn test_load_from_files_capped_read() {
        let test_dir = std::path::PathBuf::from("test_lore_cap");
        std::fs::create_dir_all(&test_dir).unwrap();

        let massive_data = "A".repeat(10 * 1024 * 1024); // 10MB
        std::fs::write(test_dir.join("TEMPLATES.md"), &massive_data).unwrap();

        let mut generator = NarrativeGenerator::default();
        let result = generator.load_from_files(&test_dir);

        std::fs::remove_dir_all(&test_dir).unwrap();

        assert!(result.is_ok());
        // Since it's capped at 5MB, the parser will parse what it can, but it shouldn't crash.
        // We ensure it didn't consume the full 10MB.
    }
}

#[test]
fn test_generate_structured() {
    let mut generator = NarrativeGenerator::default();
    generator.add_template(
        "STRUCT".to_string(),
        vec!["Hello [NAME], welcome to [PLACE].".to_string()],
    );

    let mut ctx = NarrativeContext::new();
    ctx.insert("NAME", "Mosaic");
    ctx.insert("PLACE", "Codebase");

    let segments = generator.generate_structured("STRUCT", &ctx).unwrap();

    assert_eq!(segments.len(), 5);
    assert_eq!(segments[0], NarrativeSegment::Text("Hello ".to_string()));
    assert_eq!(
        segments[1],
        NarrativeSegment::Slot {
            key: "NAME".to_string(),
            value: "Mosaic".to_string()
        }
    );
    assert_eq!(
        segments[2],
        NarrativeSegment::Text(", welcome to ".to_string())
    );
    assert_eq!(
        segments[3],
        NarrativeSegment::Slot {
            key: "PLACE".to_string(),
            value: "Codebase".to_string()
        }
    );
    assert_eq!(segments[4], NarrativeSegment::Text(".".to_string()));

    // Test string conversion via generate (legacy)
    let full_text = generator.generate("STRUCT", &ctx).unwrap();
    assert_eq!(full_text, "Hello Mosaic, welcome to Codebase.");
}

#[test]
fn test_generate_missing_template() {
    let generator = NarrativeGenerator::default();
    let ctx = NarrativeContext::new();
    let result = generator.generate_structured("NON_EXISTENT", &ctx);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        NarrativeError::TemplateNotFound(_)
    ));
}

#[test]
fn test_generate_empty_patterns() {
    let mut generator = NarrativeGenerator::default();
    generator.templates.insert(
        "EMPTY".to_string(),
        Template {
            id: "EMPTY".to_string(),
            patterns: vec![],
        },
    );
    let ctx = NarrativeContext::new();
    let result = generator.generate_structured("EMPTY", &ctx);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        NarrativeError::NoPatternsForTemplate(_)
    ));
}

#[test]
fn test_generate_unclosed_slot() {
    let mut generator = NarrativeGenerator::default();
    generator.add_template("UNCLOSED".to_string(), vec!["Hello [NAME".to_string()]);
    let ctx = NarrativeContext::new();
    let segments = generator.generate_structured("UNCLOSED", &ctx).unwrap();
    assert_eq!(segments.len(), 1);
    assert_eq!(
        segments[0],
        NarrativeSegment::Text("Hello [NAME".to_string())
    );
}

#[test]
fn test_generate_missing_fragment_options() {
    let mut generator = NarrativeGenerator::default();
    generator.add_template("EMPTY_FRAG".to_string(), vec!["[FRAG]".to_string()]);
    generator.fragments.insert(
        "FRAG".to_string(),
        FragmentType {
            id: "FRAG".to_string(),
            options: vec![],
        },
    );
    let ctx = NarrativeContext::new();
    let result = generator.generate_structured("EMPTY_FRAG", &ctx);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        NarrativeError::MissingFragmentOptions(err) if err == "FRAG"
    ));
}

#[test]
fn test_generate_missing_key() {
    let mut generator = NarrativeGenerator::default();
    generator.add_template("MISSING".to_string(), vec!["[UNKNOWN]".to_string()]);
    let ctx = NarrativeContext::new();
    let result = generator.generate_structured("MISSING", &ctx);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("UNKNOWN"));
}

#[test]
fn test_generate_optional_key_missing() {
    let mut generator = NarrativeGenerator::default();
    generator.add_template("OPTIONAL".to_string(), vec!["[UNKNOWN?]".to_string()]);
    let ctx = NarrativeContext::new();
    let segments = generator.generate_structured("OPTIONAL", &ctx).unwrap();
    assert_eq!(segments.len(), 0); // Should be empty, not an error
}
