//! Trait definitions for Pop personality quirks.

use crate::layer1::day_night::TimeOfDay;
use bevy_ecs::prelude::*;
use rand::Rng;

/// Trait enum defining possible personality quirks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
#[derive(strum_macros::EnumIter)]
pub enum Trait {
    Hacker,
    Hoarder,
    Phantom,
    MindSporeInfected,
    /// Lacks empathy, unaffected by horror.
    Psychopath,
    /// Hardened skin (+Defense/Biocompatibility).
    StoneSkin,
    /// See in the dark (+Efficiency in low light).
    NightVision,
    /// Breath underwater or in toxic atmospheres (+Biocompatibility).
    GillLungs,
    /// Blinded by bright light (-Efficiency in bright light).
    LightBlindness,
    /// Physically weak (-Health Max, +Damage taken).
    Frail,
    /// Elite combat skills but severe PTSD (Stress penalties)
    Veteran,
    /// +20% Work Speed.
    HardWorker,
    /// -20% Work Speed.
    Lazy,
    /// +20% Hunger Decay.
    Glutton,
    /// -20% Hunger Decay.
    Ascetic,
    /// +Mood at Night, -Mood at Day.
    NightOwl,
    /// +Mood at Morning, -Mood at Night.
    EarlyBird,
    /// +10% Move Speed.
    FastWalker,
    /// Hoards Valuables (Metal, Luxuries).
    Prophet,
    EngineCultist,
    Greedy,
    /// Hoards Survival Goods (Food, Meds).
    Anxious,
    /// Artistic temperament
    Artistic,
    /// Has trauma
    Trauma,
    /// Resists atmospheric hazards (+30% Biocompatibility).
    NativeBorn,
    Militaristic,
    /// Vulnerable to atmospheric hazards (-20% Biocompatibility).
    WeakImmunity,
    /// Loves fire (starts fires during breakdowns).
    Pyromaniac,
    /// Raised in the wild (+20% Move Speed, -Intellectual).
    Feral,
    /// Optimistic outlook (+Mood from Observatory).
    Optimist,
    /// Curious nature (+Knowledge/Mood from Observatory).
    Curious,
    /// Logistics expert (+Production on planets when Governor).
    LogisticsExpert,
    /// Traditional values (-Mood from Observatory).
    Traditionalist,
    Pirate,
    /// Prone to violent outbursts (+Risk of breakdown).
    Volatile,
    /// Creative mindset (+Cryo Dream rate, +Art quality).
    Creative,
    /// Intellectual mindset (+Cryo Dream rate, +Research speed).
    Intellectual,
    /// Unaffected by eating rations or corpses.
    Cannibal,
    /// Accepts survival necessities without complaint (ignore Ration mood penalty).
    Pragmatist,
    /// Socially isolated and often blamed.
    Outsider,
    /// Genetically deviant and mistrusted.
    Mutant,
    /// Highly suspicious and hostile to mutated Pops (Spec 1077).
    Xenophobic,
    /// Highly empathetic and prone to guilt.
    Compassionate,
    /// Sensitive to The Hum (Spec 238).
    Sensitive,
    /// Specialized trait for farming.
    GreenThumb,
    /// Specialized trait for administration and diplomacy.
    SilverTongue,
    /// Specialized trait for mining/underground work.
    MoleEyes,
    /// Specialized trait for hauling.
    Hunchback,
    /// Specialized trait for engineering.
    StaticSkin,
    /// Manufactured in a Clone Vat.
    Clone,
    /// Reduced social needs.
    Soulless,
    /// Resistant to Void Stare effects (Spec 216).
    VoidTouched,
    /// Highly susceptible to Void Stare effects (Spec 216).
    Agoraphobic,
    /// (Spec 250) Obsessed with augmenting their body.
    Transhumanist,
    /// Noble scion, refuses manual labor but pays allowance (Spec 263).
    Noble,
    /// Distrusts colony authorities, ignores placebos (Spec 256).
    Distrustful,
    /// Experiences light colors as emotional sounds.
    Synesthete,
    /// Leaves a spiteful will upon death, giving belongings to rivals or pets.
    Spiteful,
    /// Biosphere empathy link, harmonizes stress and works better near flora.
    EmpathicLink,
    /// Formal administrative capabilities. Understood the bureaucracy (Spec 464).
    Bureaucrat,
    /// Spec 1261: Eliminates rest, increases work speed, doubles stress.
    InsomniaDrive,
    /// (Spec 472) Basic synthetic pop. 100% work efficiency, no morale needs, apathetic to emergencies.
    Synth,
    /// Synthesizes food from light.
    Photosynthesis,
    /// Thick hide that protects against damage.
    ThickSkin,
    /// Fragile bones.
    BrittleBones,
    /// Needs more food than normal.
    ExtremeHunger,
    /// Pop actively dissents against the colony (Spec 985).
    Dissident,
    #[cfg(feature = "nova")]
    /// Yearns for the past or the core worlds, refusing new work but gaining comfort from the past.
    Homesick,
    /// Has fled from a warzone.
    Refugee,
    /// Suffered extreme trauma.
    Traumatized,
    /// Braver than most: resists panic cascades (Spec 1371).
    Courage,
}

impl Trait {
    /// Returns a human-readable label for the trait.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Psychopath => "Psychopath",
            Self::Militaristic => "Militaristic",
            Self::StoneSkin => "Stone Skin",
            Self::NightVision => "Night Vision",
            Self::GillLungs => "Gill Lungs",
            Self::LightBlindness => "Light Blindness",
            Self::Frail => "Frail",
            Self::HardWorker => "Hard Worker",
            Self::Lazy => "Lazy",
            Self::Glutton => "Glutton",
            Self::Ascetic => "Ascetic",
            Self::NightOwl => "Night Owl",
            Self::EarlyBird => "Early Bird",
            Self::FastWalker => "Fast Walker",
            Self::Prophet => "Prophet",
            Self::EngineCultist => "Engine Cultist",
            Self::Greedy => "Greedy",
            Self::Anxious => "Anxious",
            Self::NativeBorn => "Native Born",
            Self::WeakImmunity => "Weak Immunity",
            Self::Pyromaniac => "Pyromaniac",
            Self::Feral => "Feral",
            Self::Optimist => "Optimist",
            Self::Curious => "Curious",
            Self::Traditionalist => "Traditionalist",
            Self::Pirate => "Pirate",
            Self::Volatile => "Volatile",
            Self::Creative => "Creative",
            Self::Intellectual => "Intellectual",
            Self::Cannibal => "Cannibal",
            Self::Pragmatist => "Pragmatist",
            Self::Outsider => "Outsider",
            Self::Mutant => "Mutant",
            Self::Xenophobic => "Xenophobic",
            Self::Compassionate => "Compassionate",
            Self::Sensitive => "Sensitive",
            Self::GreenThumb => "GreenThumb",
            Self::SilverTongue => "SilverTongue",
            Self::MoleEyes => "MoleEyes",
            Self::Hunchback => "Hunchback",
            Self::StaticSkin => "StaticSkin",
            Self::Clone => "Clone",
            Self::Soulless => "Soulless",
            Self::VoidTouched => "Void Touched",
            Self::Agoraphobic => "Agoraphobic",
            Self::Transhumanist => "Transhumanist",
            Self::Noble => "Noble",
            Self::Distrustful => "Distrustful",

            Self::Synesthete => "Synesthete",
            Self::Spiteful => "Spiteful",
            Self::EmpathicLink => "Empathic Link",
            Self::Bureaucrat => "Bureaucrat",
            Self::Synth => "Synthetic",
            Self::LogisticsExpert => "Logistics Expert",
            Self::Photosynthesis => "Photosynthesis",
            Self::ThickSkin => "Thick Skin",
            Self::BrittleBones => "Brittle Bones",
            Self::ExtremeHunger => "Extreme Hunger",
            Self::Dissident => "Dissident",
            #[cfg(feature = "nova")]
            Trait::Homesick => "Homesick",
            Self::Refugee => "Refugee",
            Self::Traumatized => "Traumatized",
            Self::InsomniaDrive => "Insomnia Drive",
            Self::Veteran => "Veteran",
            Self::Phantom => "Phantom",
            Self::Hoarder => "Hoarder",
            Self::MindSporeInfected => "Mind-Spore Infected",
            Self::Artistic => "Artistic",
            Self::Trauma => "Trauma",
            Self::Hacker => "Hacker",
            Self::Courage => "Courageous",
        }
    }
}

/// Component storing a set of traits for a pop.
#[derive(Component, Debug, Clone, Default)]
pub struct Traits(pub bevy::utils::HashSet<Trait>);

impl Traits {
    /// Checks if the pop has the given trait.
    #[must_use]
    pub fn has(&self, t: Trait) -> bool {
        self.0.contains(&t)
    }

    /// Adds a trait to the set.
    pub fn add(&mut self, t: Trait) {
        self.0.insert(t);
    }

    /// Removes a trait from the set.
    pub fn remove(&mut self, t: Trait) {
        self.0.remove(&t);
    }

    /// Iterator over the traits.
    pub fn iter(&self) -> impl Iterator<Item = Trait> + '_ {
        use strum::IntoEnumIterator;
        Trait::iter().filter(move |t| self.has(*t))
    }

    /// Generates a random set of traits.
    pub fn random<R: Rng>(rng: &mut R) -> Self {
        let mut traits = Traits::default();
        // Simple logic: 50% chance to get 1 trait, 20% for 2.
        let count = if rng.gen_bool(0.2) {
            2
        } else {
            usize::from(rng.gen_bool(0.5))
        };

        // Pool of all traits
        let pool = [
            Trait::HardWorker,
            Trait::Lazy,
            Trait::Glutton,
            Trait::Ascetic,
            Trait::NightOwl,
            Trait::EarlyBird,
            Trait::FastWalker,
            Trait::Greedy,
            Trait::Anxious,
            Trait::NativeBorn,
            Trait::WeakImmunity,
            Trait::Pyromaniac,
            Trait::Optimist,
            Trait::Curious,
            Trait::Traditionalist,
            Trait::Volatile,
            Trait::Creative,
            Trait::Intellectual,
            #[cfg(feature = "nova")]
            Trait::Homesick,
            Trait::Cannibal,
            Trait::Pragmatist,
            Trait::Outsider,
            Trait::Mutant,
            Trait::Xenophobic,
            Trait::Compassionate,
            Trait::Synesthete,
            Trait::Spiteful,
            Trait::EmpathicLink,
            Trait::Bureaucrat,
            Trait::InsomniaDrive,
            Trait::Courage,
        ];

        let mut added = 0;
        while added < count {
            let t = pool[rng.gen_range(0..pool.len())];

            // Check conflicts
            if t == Trait::HardWorker && traits.has(Trait::Lazy) {
                continue;
            }
            if t == Trait::Lazy && traits.has(Trait::HardWorker) {
                continue;
            }
            if t == Trait::Glutton && traits.has(Trait::Ascetic) {
                continue;
            }
            if t == Trait::Ascetic && traits.has(Trait::Glutton) {
                continue;
            }
            if t == Trait::NightOwl && traits.has(Trait::EarlyBird) {
                continue;
            }
            if t == Trait::EarlyBird && traits.has(Trait::NightOwl) {
                continue;
            }
            if t == Trait::NativeBorn && traits.has(Trait::WeakImmunity) {
                continue;
            }
            if t == Trait::WeakImmunity && traits.has(Trait::NativeBorn) {
                continue;
            }
            if t == Trait::Optimist && traits.has(Trait::Anxious) {
                continue;
            }
            if t == Trait::Anxious && traits.has(Trait::Optimist) {
                continue;
            }
            if t == Trait::Curious && traits.has(Trait::Traditionalist) {
                continue;
            }
            if t == Trait::Traditionalist && traits.has(Trait::Curious) {
                continue;
            }
            if t == Trait::Spiteful && traits.has(Trait::Compassionate) {
                continue;
            }
            if t == Trait::Compassionate && traits.has(Trait::Spiteful) {
                continue;
            }
            if t == Trait::Courage && traits.has(Trait::Anxious) {
                continue;
            }
            if t == Trait::Anxious && traits.has(Trait::Courage) {
                continue;
            }
            if t == Trait::Courage && traits.has(Trait::Agoraphobic) {
                continue;
            }
            if t == Trait::Agoraphobic && traits.has(Trait::Courage) {
                continue;
            }

            if !traits.has(t) {
                traits.add(t);
                added += 1;
            }
        }

        traits
    }
}

use crate::layer1::utility_types::AssignmentType;

/// Returns the job efficiency modifier based on specialization traits.
#[must_use]
pub fn get_job_efficiency_modifier(traits: &Traits, job: AssignmentType) -> f32 {
    let mut modifier = 1.0;

    if traits.has(Trait::GreenThumb) {
        if job == AssignmentType::FarmWorker {
            modifier += 0.2;
        } else {
            modifier -= 0.2;
        }
    }

    if traits.has(Trait::SilverTongue) {
        if job == AssignmentType::Administrator {
            modifier += 0.2;
        } else {
            modifier -= 0.2;
        }
    }

    if traits.has(Trait::MoleEyes) {
        if job == AssignmentType::DeepMining {
            modifier += 1.0;
        } else {
            modifier -= 0.5;
        }
    }

    if traits.has(Trait::Hunchback) {
        // No bonus assigned yet, apply penalty to everything
        modifier -= 0.2;
    }

    if traits.has(Trait::StaticSkin) {
        // No bonus assigned yet, apply penalty to everything
        modifier -= 0.2;
    }

    modifier
}

/// Returns the work speed modifier from traits.
#[must_use]
pub fn get_trait_work_speed_modifier(traits: &Traits) -> f32 {
    let mut modifier = 1.0;
    if traits.has(Trait::InsomniaDrive) {
        modifier += 0.3;
    }
    if traits.has(Trait::MindSporeInfected) {
        modifier += 0.5;
    }
    if traits.has(Trait::HardWorker) {
        modifier += 0.2;
    }
    if traits.has(Trait::Lazy) {
        modifier -= 0.2;
    }
    if traits.has(Trait::EngineCultist) {
        modifier += 0.5;
    }
    if traits.has(Trait::Pirate) {
        modifier -= 0.3;
    }
    #[cfg(feature = "nova")]
    if traits.has(Trait::Homesick) {
        modifier -= 0.8;
    }
    modifier
}

/// Returns the hunger decay modifier from traits.
#[must_use]
pub fn get_trait_hunger_decay_modifier(traits: &Traits) -> f32 {
    let mut modifier = 1.0;
    if traits.has(Trait::Glutton) {
        modifier += 0.2;
    }
    if traits.has(Trait::Ascetic) {
        modifier -= 0.2;
    }
    modifier
}

/// Returns the leisure decay modifier from traits.
#[must_use]
pub fn get_trait_leisure_decay_modifier(traits: &Traits) -> f32 {
    let mut modifier = 1.0;
    if traits.has(Trait::InsomniaDrive) {
        modifier += 1.0;
    }
    if traits.has(Trait::MindSporeInfected) {
        modifier += 0.5;
        modifier += 1.0;
    }
    if traits.has(Trait::Synth) {
        return 0.0;
    }
    if traits.has(Trait::Soulless) {
        modifier -= 0.5;
    }
    if traits.has(Trait::Noble) {
        modifier += 0.5;
    }
    modifier
}

/// Returns the movement speed modifier from traits.
#[must_use]
pub fn get_trait_move_speed_modifier(traits: &Traits) -> f32 {
    let mut modifier = 1.0;
    if traits.has(Trait::FastWalker) {
        modifier += 0.1;
    }
    if traits.has(Trait::Feral) {
        modifier += 0.2;
    }
    modifier
}

/// Returns the mood modifier from traits based on time of day.
#[must_use]
pub fn get_trait_mood_modifier(traits: &Traits, time_of_day: TimeOfDay) -> f32 {
    let mut modifier = 0.0;
    if traits.has(Trait::NightOwl) {
        match time_of_day {
            TimeOfDay::Night => modifier += 0.1,
            TimeOfDay::Day => modifier -= 0.05,
            TimeOfDay::Dawn | TimeOfDay::Dusk => {}
        }
    }
    if traits.has(Trait::EarlyBird) {
        match time_of_day {
            TimeOfDay::Dawn | TimeOfDay::Day => modifier += 0.05,
            TimeOfDay::Night => modifier -= 0.1,
            TimeOfDay::Dusk => {}
        }
    }
    modifier
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer1::day_night::TimeOfDay;

    use crate::layer1::utility_types::AssignmentType;

    #[test]
    fn test_job_efficiency_modifiers() {
        let green_thumb = {
            let mut t = Traits::default();
            t.add(Trait::GreenThumb);
            t
        };
        let silver_tongue = {
            let mut t = Traits::default();
            t.add(Trait::SilverTongue);
            t
        };
        let mole_eyes = {
            let mut t = Traits::default();
            t.add(Trait::MoleEyes);
            t
        };
        let hunchback = {
            let mut t = Traits::default();
            t.add(Trait::Hunchback);
            t
        };
        let static_skin = {
            let mut t = Traits::default();
            t.add(Trait::StaticSkin);
            t
        };
        let normal = Traits::default();

        // Normal has no modifiers
        assert!(
            (get_job_efficiency_modifier(&normal, AssignmentType::FarmWorker) - 1.0).abs()
                < f32::EPSILON
        );

        // GreenThumb bonuses and penalties
        assert!(get_job_efficiency_modifier(&green_thumb, AssignmentType::FarmWorker) > 1.0);
        assert!(get_job_efficiency_modifier(&green_thumb, AssignmentType::Administrator) < 1.0);

        // SilverTongue bonuses and penalties
        assert!(get_job_efficiency_modifier(&silver_tongue, AssignmentType::Administrator) > 1.0);
        assert!(get_job_efficiency_modifier(&silver_tongue, AssignmentType::FarmWorker) < 1.0);

        // MoleEyes bonuses and penalties
        assert!(get_job_efficiency_modifier(&mole_eyes, AssignmentType::DeepMining) > 1.5);
        assert!(get_job_efficiency_modifier(&mole_eyes, AssignmentType::FarmWorker) < 1.0);

        // Hunchback penalties (no bonus assigned to AssignmentType yet)
        assert!(get_job_efficiency_modifier(&hunchback, AssignmentType::Administrator) < 1.0);

        // StaticSkin penalties (no bonus assigned to AssignmentType yet)
        assert!(get_job_efficiency_modifier(&static_skin, AssignmentType::FarmWorker) < 1.0);
    }

    #[test]
    fn test_traits_random_generation() {
        let mut rng = rand::thread_rng();
        // Just verify it doesn't panic and produces valid Traits
        let _ = Traits::random(&mut rng);
    }

    #[test]
    fn test_work_speed_modifiers() {
        let hard_worker = {
            let mut t = Traits::default();
            t.add(Trait::HardWorker);
            t
        };
        let lazy = {
            let mut t = Traits::default();
            t.add(Trait::Lazy);
            t
        };
        let normal = Traits::default();

        assert!(
            get_trait_work_speed_modifier(&hard_worker) > 1.0,
            "HardWorker should work faster"
        );
        assert!(
            get_trait_work_speed_modifier(&lazy) < 1.0,
            "Lazy should work slower"
        );
        assert!(
            (get_trait_work_speed_modifier(&normal) - 1.0).abs() < f32::EPSILON,
            "Normal should work at normal speed"
        );
        let pirate = {
            let mut t = Traits::default();
            t.add(Trait::Pirate);
            t
        };
        assert!(
            get_trait_work_speed_modifier(&pirate) < 1.0,
            "Pirate should work slower"
        );
    }

    #[test]
    fn test_hunger_decay_modifiers() {
        let glutton = {
            let mut t = Traits::default();
            t.add(Trait::Glutton);
            t
        };
        let ascetic = {
            let mut t = Traits::default();
            t.add(Trait::Ascetic);
            t
        };
        let normal = Traits::default();

        assert!(
            get_trait_hunger_decay_modifier(&glutton) > 1.0,
            "Glutton should eat more"
        );
        assert!(
            get_trait_hunger_decay_modifier(&ascetic) < 1.0,
            "Ascetic should eat less"
        );
        assert!(
            (get_trait_hunger_decay_modifier(&normal) - 1.0).abs() < f32::EPSILON,
            "Normal should eat normally"
        );
    }

    #[test]
    fn test_leisure_decay_modifiers() {
        let soulless = {
            let mut t = Traits::default();
            t.add(Trait::Soulless);
            t
        };
        let noble = {
            let mut t = Traits::default();
            t.add(Trait::Noble);
            t
        };
        let synth = {
            let mut t = Traits::default();
            t.add(Trait::Synth);
            t
        };
        let normal = Traits::default();

        assert!(
            get_trait_leisure_decay_modifier(&soulless) < 1.0,
            "Soulless should have reduced leisure decay"
        );
        assert!(
            get_trait_leisure_decay_modifier(&noble) > 1.0,
            "Noble should have increased leisure decay"
        );
        assert!(
            (get_trait_leisure_decay_modifier(&synth) - 0.0).abs() < f32::EPSILON,
            "Synth should have zero leisure decay"
        );
        assert!(
            (get_trait_leisure_decay_modifier(&normal) - 1.0).abs() < f32::EPSILON,
            "Normal should decay normally"
        );
    }

    #[test]
    fn test_feral_speed_modifier() {
        let feral = {
            let mut t = Traits::default();
            t.add(Trait::Feral);
            t
        };
        let fast = {
            let mut t = Traits::default();
            t.add(Trait::FastWalker);
            t
        };
        let both = {
            let mut t = Traits::default();
            t.add(Trait::Feral);
            t.add(Trait::FastWalker);
            t
        };

        assert!(
            (get_trait_move_speed_modifier(&feral) - 1.2).abs() < 0.0001,
            "Feral should be 20% faster"
        );
        assert!(
            (get_trait_move_speed_modifier(&fast) - 1.1).abs() < 0.0001,
            "FastWalker should be 10% faster"
        );
        assert!(
            (get_trait_move_speed_modifier(&both) - 1.3).abs() < 0.0001,
            "Both should be 30% faster"
        );
    }

    #[test]
    fn test_night_owl_mood_modifier() {
        let night_owl = {
            let mut t = Traits::default();
            t.add(Trait::NightOwl);
            t
        };

        let mood_night = get_trait_mood_modifier(&night_owl, TimeOfDay::Night);
        assert!(mood_night > 0.0, "NightOwl should be happier at night");

        let mood_day = get_trait_mood_modifier(&night_owl, TimeOfDay::Day);
        assert!(mood_day < 0.0, "NightOwl should be sadder during day");
    }

    #[test]
    fn test_conflicting_traits() {
        let mut rng = rand::thread_rng();
        for _ in 0..100 {
            let traits = Traits::random(&mut rng);
            let has_lazy = traits.has(Trait::Lazy);
            let has_hard_worker = traits.has(Trait::HardWorker);
            assert!(
                !(has_lazy && has_hard_worker),
                "Should not be both Lazy and HardWorker"
            );

            let has_glutton = traits.has(Trait::Glutton);
            let has_ascetic = traits.has(Trait::Ascetic);
            assert!(
                !(has_glutton && has_ascetic),
                "Should not be both Glutton and Ascetic"
            );

            let has_night_owl = traits.has(Trait::NightOwl);
            let has_early_bird = traits.has(Trait::EarlyBird);
            assert!(
                !(has_night_owl && has_early_bird),
                "Should not be both NightOwl and EarlyBird"
            );
        }
    }

    #[test]
    fn test_insomnia_drive_increases_productivity() {
        let mut traits = Traits::default();
        traits.add(Trait::InsomniaDrive);

        let modifier = get_trait_work_speed_modifier(&traits);
        assert!(
            (modifier - 1.3).abs() < f32::EPSILON,
            "Insomnia Drive should increase work speed by 30%"
        );
    }

    #[test]
    fn test_insomnia_drive_doubles_stress_generation() {
        let mut traits = Traits::default();
        traits.add(Trait::InsomniaDrive);

        let modifier = get_trait_leisure_decay_modifier(&traits);
        assert!(
            (modifier - 2.0).abs() < f32::EPSILON,
            "Insomnia Drive should increase leisure decay by 100%"
        );
    }
}
