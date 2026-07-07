//! Wave content as pure data. Difficulty lives entirely in `WAVES`; the spawn
//! systems in `enemy.rs` just play this schedule back. See SPEC.md for the
//! design rationale behind the numbers.

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EnemyArchetype {
    Grunt,
    Runner,
    Brute,
}

pub struct ArchetypeStats {
    pub hp: f32,
    pub speed: f32,
    pub leak_cost: u32,
    pub scale: f32,
}

impl EnemyArchetype {
    pub const fn stats(self) -> ArchetypeStats {
        match self {
            EnemyArchetype::Grunt => ArchetypeStats {
                hp: 100.0,
                speed: 2.0,
                leak_cost: 1,
                scale: 1.0,
            },
            EnemyArchetype::Runner => ArchetypeStats {
                hp: 50.0,
                speed: 4.0,
                leak_cost: 1,
                scale: 0.7,
            },
            EnemyArchetype::Brute => ArchetypeStats {
                hp: 400.0,
                speed: 1.2,
                leak_cost: 3,
                scale: 1.6,
            },
        }
    }
}

/// One scheduled batch inside a wave: `count` enemies of one archetype,
/// starting `start_delay` seconds into the wave, one every `interval` seconds.
pub struct SpawnGroup {
    pub archetype: EnemyArchetype,
    pub count: u32,
    pub start_delay: f32,
    pub interval: f32,
}

pub struct WaveDef {
    pub groups: &'static [SpawnGroup],
    /// Materials granted when the wave is fully resolved.
    pub reward: u32,
}

const fn group(archetype: EnemyArchetype, count: u32, start_delay: f32, interval: f32) -> SpawnGroup {
    SpawnGroup {
        archetype,
        count,
        start_delay,
        interval,
    }
}

use EnemyArchetype::{Brute, Grunt, Runner};

pub const WAVES: &[WaveDef] = &[
    // 1
    WaveDef {
        groups: &[group(Grunt, 5, 0.0, 1.5)],
        reward: 40,
    },
    // 2
    WaveDef {
        groups: &[group(Grunt, 5, 0.0, 1.2), group(Grunt, 5, 6.0, 1.2)],
        reward: 50,
    },
    // 3: runners debut
    WaveDef {
        groups: &[group(Grunt, 6, 0.0, 1.2), group(Runner, 4, 4.0, 0.8)],
        reward: 55,
    },
    // 4
    WaveDef {
        groups: &[group(Grunt, 8, 0.0, 1.0), group(Runner, 6, 5.0, 0.7)],
        reward: 65,
    },
    // 5: brutes debut
    WaveDef {
        groups: &[group(Brute, 1, 0.0, 1.0), group(Grunt, 8, 2.0, 1.0)],
        reward: 75,
    },
    // 6
    WaveDef {
        groups: &[
            group(Brute, 2, 0.0, 6.0),
            group(Runner, 8, 3.0, 0.6),
            group(Grunt, 6, 8.0, 1.0),
        ],
        reward: 80,
    },
    // 7
    WaveDef {
        groups: &[group(Grunt, 12, 0.0, 0.8), group(Runner, 8, 6.0, 0.5)],
        reward: 90,
    },
    // 8
    WaveDef {
        groups: &[group(Brute, 3, 0.0, 5.0), group(Grunt, 10, 3.0, 0.8)],
        reward: 100,
    },
    // 9
    WaveDef {
        groups: &[
            group(Runner, 10, 0.0, 0.5),
            group(Brute, 2, 4.0, 6.0),
            group(Grunt, 10, 8.0, 0.8),
        ],
        reward: 110,
    },
    // 10: brute pairs sandwich a dense rush
    WaveDef {
        groups: &[
            group(Brute, 2, 0.0, 4.0),
            group(Grunt, 12, 2.0, 0.6),
            group(Runner, 10, 8.0, 0.4),
            group(Brute, 2, 14.0, 4.0),
        ],
        reward: 0,
    },
];
