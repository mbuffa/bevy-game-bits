//! Electricity: turbines and solar panels feed a single global `PowerGrid`
//! pool; light posts and coolers draw from it, and batteries bank the
//! surplus to cover a deficit later on. No circuits/wiring yet — one shared
//! budget map-wide, `tick_power` the single system that recomputes it (and
//! every producer's live `PowerOutput`) each sim tick. See
//! `docs/07-add-power.md`.

use bevy::prelude::*;
use bevy_ecs_ldtk::prelude::GridCoords;

use crate::config::*;
use crate::construction::{Battery, Cooler, SolarPanel, Turbine};
use crate::daynight::{self, GameClock};
use crate::map::RoofMap;
use crate::temperature::{self, Room, TemperatureMap};
use crate::weather::{self, Wind, WindBand, WindExposureMap};
use crate::zones::ZoneRegion;

/// The colony's single shared electricity pool — recomputed wholesale every
/// sim tick by `tick_power`; nothing else writes it.
#[derive(Resource, Default)]
pub struct PowerGrid {
    /// Live sum of every producer's `PowerOutput` this tick, units/sec.
    pub production: f32,
    /// Live sum of every consumer's *desired* draw this tick, units/sec — a
    /// light post only wants power while trying to be lit (night).
    pub consumption: f32,
    /// Sum of every `Battery.charge`, units.
    pub stored: f32,
    /// Sum of every battery's capacity, units.
    pub capacity: f32,
    /// How much of `consumption` the grid actually covered this tick, in
    /// 0..=1 — production topped up by battery discharge. Consumers
    /// (`construction::sync_lightposts`) scale their output by this, so an
    /// under-supplied grid dims everything together instead of some going
    /// dark while others stay fully lit.
    pub powered_fraction: f32,
}

/// Live output of a producer (turbine/solar panel), units/sec — written by
/// `tick_power`, read by the info panel and the debug Power tab.
#[derive(Component, Default)]
pub struct PowerOutput(pub f32);

/// A consumer's desired draw at full demand, units/sec — the light post's is
/// always `LIGHTPOST_POWER_DRAW`; `tick_power` scales by how much it
/// currently wants power (light posts only want it at night) before summing
/// into `PowerGrid::consumption`.
#[derive(Component)]
pub struct PowerConsumer {
    pub demand: f32,
}

/// Recompute the whole grid in one pass — producer output (wind/sun
/// modulated), consumer demand, and battery charge/discharge — so every
/// reader (`PowerGrid`, every `PowerOutput`, every `Battery.charge`) stays
/// consistent within the same frame. Sim-gated (`game.rs`), chained right
/// after `weather::update_wind`/`update_wind_exposure` so it reads this
/// frame's wind; `GameClock` is one frame stale here, the same lag
/// `construction::aim_solar_panels`/`sync_lightposts` already tolerate.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn tick_power(
    time: Res<Time>,
    clock: Res<GameClock>,
    wind: Res<Wind>,
    exposure: Res<WindExposureMap>,
    roofs: Res<RoofMap>,
    temps: Res<TemperatureMap>,
    mut grid: ResMut<PowerGrid>,
    mut turbines: Query<(&GridCoords, &mut PowerOutput), (With<Turbine>, Without<SolarPanel>)>,
    mut solar_panels: Query<(&GridCoords, &mut PowerOutput), (With<SolarPanel>, Without<Turbine>)>,
    consumers: Query<&PowerConsumer>,
    coolers: Query<(&GridCoords, &Cooler)>,
    rooms: Query<(&ZoneRegion, &Room)>,
    mut batteries: Query<&mut Battery>,
) {
    let dt = time.delta_secs();

    // Turbines: peak rate scaled by the wind actually reaching the rotor
    // (ambient x per-cell ALTITUDE exposure — a rotor sits well above a
    // wall's shelter, only tall blockers like tree canopies reach it),
    // gated off entirely below the cut-in — a stalled rotor produces
    // nothing, matching `spin_turbines`'s visual.
    let mut production = 0.0;
    for (cell, mut output) in &mut turbines {
        let factor = if wind.current > POWER_WIND_CUTIN {
            (exposure.wind_at(*cell, &wind, WindBand::Altitude).length() / POWER_WIND_FULL_STRENGTH)
                .min(1.0)
        } else {
            0.0
        };
        output.0 = TURBINE_PEAK_OUTPUT_PER_SECOND * factor;
        production += output.0;
    }

    // Solar panels: peak rate scaled by a noon-peaking bell curve (daylight
    // x sun elevation — see `daynight::sun_height`), zeroed outright under a
    // roof. Anchor cell only; the 2x2 footprint doesn't split hairs.
    for (cell, mut output) in &mut solar_panels {
        let light_factor = clock.daylight() * daynight::sun_height(clock.day_progress());
        let outdoors = weather::is_outdoors(*cell, &roofs);
        output.0 = if outdoors {
            SOLAR_PEAK_OUTPUT_PER_SECOND * light_factor
        } else {
            0.0
        };
        production += output.0;
    }

    // Consumers: a light post only wants power while trying to be lit.
    let light_desire = 1.0 - clock.daylight();
    let mut consumption: f32 = consumers.iter().map(|c| c.demand * light_desire).sum();

    // Coolers: draw scales with how far their room sits from the target —
    // nothing at the setpoint, capped at `CLIMATE_POWER_DRAW` once the gap
    // reaches `CLIMATE_FULL_DELTA`. A freestanding cooler (no indoor
    // neighbour, `cooler_room_temperature` returns `None`) has no room to
    // condition and draws nothing.
    for (cell, cooler) in &coolers {
        let desire = match temperature::cooler_room_temperature(*cell, &temps, &roofs, &rooms) {
            Some(room_temp) => {
                ((room_temp - cooler.target_c).abs() / CLIMATE_FULL_DELTA).clamp(0.0, 1.0)
            }
            None => 0.0,
        };
        consumption += CLIMATE_POWER_DRAW * desire;
    }

    // Batteries: bank the surplus, drain to cover a deficit, each capped at
    // `BATTERY_MAX_FLOW` per battery so a big surplus/deficit ramps visibly
    // instead of snapping a battery full/empty in one tick.
    let net = production - consumption;
    let mut discharged = 0.0;
    if net > 0.0 {
        let mut budget = net * dt;
        for mut battery in &mut batteries {
            if budget <= 0.0 {
                break;
            }
            let room = (BATTERY_CAPACITY - battery.charge).max(0.0);
            let take = budget.min(room).min(BATTERY_MAX_FLOW * dt);
            battery.charge += take;
            budget -= take;
        }
    } else if net < 0.0 {
        let mut deficit = -net * dt;
        for mut battery in &mut batteries {
            if deficit <= 0.0 {
                break;
            }
            let available = battery.charge.min(BATTERY_MAX_FLOW * dt);
            let take = deficit.min(available);
            battery.charge -= take;
            discharged += take;
            deficit -= take;
        }
    }

    // Coverage: what fraction of this tick's demand was actually met, by
    // production alone or topped up by discharge — batteries empty and no
    // surplus, this reduces to the plain `production / consumption` ratio.
    let consumption_dt = consumption * dt;
    grid.powered_fraction = if consumption_dt > 0.0 {
        (production * dt + discharged).min(consumption_dt) / consumption_dt
    } else {
        1.0
    };
    grid.production = production;
    grid.consumption = consumption;
    grid.stored = batteries.iter().map(|battery| battery.charge).sum();
    grid.capacity = batteries.iter().len() as f32 * BATTERY_CAPACITY;
}
