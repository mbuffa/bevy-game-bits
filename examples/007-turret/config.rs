use bevy::prelude::*;
use std::f32::consts::{FRAC_PI_3, PI};

// Play field lies on the XZ plane, centered on the origin.
// Original prototype was 800x600 px; world units are px / 20.
pub const FIELD_WIDTH: f32 = 40.0;
pub const FIELD_DEPTH: f32 = 30.0;

pub const ENEMY_RADIUS: f32 = 1.0;
pub const ENEMY_SPEED: f32 = 2.0;
pub const ENEMY_MAX_HP: f32 = 100.0;

pub const WAVE_INTERVAL: f32 = 2.0;
pub const WAVE_MIN_ENEMIES: u32 = 1;
pub const WAVE_MAX_ENEMIES: u32 = 9;

pub const PROJECTILE_SPEED: f32 = 20.0;
pub const PROJECTILE_RADIUS: f32 = 0.08;

pub const MINIGUN_SHOT_INTERVAL: f32 = 0.1; // 10 rounds/s while aligned
pub const MINIGUN_DAMAGE: f32 = 5.0;
pub const TRACER_EVERY: u32 = 4; // every Nth round is a visible tracer

pub const LASER_DPS: f32 = 120.0;

pub const SENSOR_RANGE: f32 = 24.0;
pub const CONE_HALF_ANGLE: f32 = 8.0 * PI / 180.0;
pub const SWEEP_LIMIT: f32 = FRAC_PI_3; // +/- 60 degrees around facing
pub const SWEEP_SPEED: f32 = 65.0 * PI / 180.0;
pub const GUN_TURN_SPEED: f32 = 65.0 * PI / 180.0;
pub const ALIGN_THRESHOLD: f32 = 0.05; // ~3 degrees

pub const BASE_HEIGHT: f32 = 0.6;
pub const GUN_HEIGHT: f32 = 0.9;
pub const SENSOR_HEIGHT: f32 = 1.3;
pub const BARREL_LENGTH: f32 = 1.4;

pub const FLASH_DURATION: f32 = 0.1;

pub const CLEAR_COLOR: Color = Color::srgb(0.05, 0.07, 0.12);
pub const GROUND_COLOR: Color = Color::srgb(0.22, 0.38, 0.85);
pub const ENEMY_COLOR: Color = Color::srgb(0.75, 0.15, 0.25);
pub const KINETIC_BASE_COLOR: Color = Color::srgb(0.35, 0.38, 0.42);
pub const LASER_BASE_COLOR: Color = Color::srgb(0.16, 0.55, 0.55);
pub const BARREL_COLOR: Color = Color::srgb(0.15, 0.15, 0.18);
pub const SENSOR_COLOR: Color = Color::srgb(0.95, 0.85, 0.2);
pub const PROJECTILE_COLOR: Color = Color::srgb(1.0, 0.6, 0.1);
pub const TRACER_LIGHT_COLOR: Color = Color::srgb(1.0, 0.55, 0.15);
pub const LASER_BEAM_COLOR: Color = Color::srgb(1.0, 0.2, 0.2);
pub const CONE_SEARCH_COLOR: Color = Color::srgba(0.3, 1.0, 0.4, 0.6);
pub const CONE_LOCKED_COLOR: Color = Color::srgba(1.0, 0.6, 0.15, 0.8);
