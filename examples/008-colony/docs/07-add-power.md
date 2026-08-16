# Add electricity (producers, consumers, storage)

Worked example: `power.rs` — the whole grid. Unlike the other guides, this
one isn't about plugging into an existing pipeline; it *is* the pipeline. Read
it before adding a new power-producing building, a new power-consuming
building, or a new way to store/move power.

The model, end to end:

```
Turbine/SolarPanel (PowerOutput) --\
                                     >-- power::tick_power --> PowerGrid
Lightpost (PowerConsumer)      --/         |
                                            v
                                  Battery.charge (bank surplus,
                                  discharge to cover a deficit)
```

One global pool (`PowerGrid`), no circuits/wiring — every producer feeds it,
every consumer draws from it, map-wide. `power::tick_power` is the *only*
system that writes `PowerGrid`, any `PowerOutput`, or any `Battery.charge`;
everything else only reads. It's sim-gated and chained right after
`weather::update_wind`/`update_wind_exposure` (`game.rs`), so it always sees
this frame's wind; `GameClock` is one frame stale there, the same lag
`construction::aim_solar_panels`/`sync_lightposts` already tolerate.

## Checklist

### Adding a producer (like `Turbine`/`SolarPanel`)

1. Give the built entity a `power::PowerOutput` component (starts at
   `PowerOutput::default()`, i.e. `0.0`) in `construction::complete_builds`'
   arm for your `BuildableKind` (and in any other LDtk/devtool spawn site
   that builds one pre-completed).
2. Add a query + a factor to `power::tick_power`: read whatever live state
   your output should scale with (`Wind`/`WindExposureMap` for wind,
   `GameClock`/`RoofMap` for sun — see `daynight::sun_height` for a
   noon-peaking curve), write `output.0`, add it into the local `production`
   sum.
3. A daily energy budget (`config.rs`'s `SOLAR_OUTPUT_PER_DAY`/
   `TURBINE_OUTPUT_PER_DAY` pattern) converts cleanly to a peak units/second
   rate by dividing by `DAY_LENGTH_SECS` (day-only producer) or
   `CYCLE_LENGTH_SECS` (round-the-clock producer) — keeps "N/day", "M
   stored", and "1/sec" comparable at a glance.
4. The info panel (`ui::update_panel`) and the debug Power tab
   (`debug_ui::update_power_tab`) already read `PowerOutput` generically —
   no changes needed there unless you want a producer-specific line.

### Adding a consumer (like `Lightpost`)

1. Give the built entity a `power::PowerConsumer { demand }` component in
   `complete_builds` — `demand` is the units/sec it wants at *full* draw.
2. In `power::tick_power`, decide how much of that demand it wants *this
   tick* (a light post only wants it at night, `1.0 - clock.daylight()`) and
   fold `consumer.demand * desire` into the `consumption` sum. If your
   consumer's desire is always 1.0, just sum `consumer.demand` directly.
3. Read `PowerGrid::powered_fraction` (0..=1, how much of `consumption` the
   grid actually covered) in whatever system drives your consumer's visible
   behavior — `construction::sync_lightposts` is the template: `lit =
   (1.0 - clock.daylight()) * grid.powered_fraction`. This is what makes an
   under-supplied grid dim every consumer proportionally instead of some
   staying lit while others go fully dark.

### Adding storage (like `Battery`)

A battery is a full `BuildableKind` (see
[02-add-a-building.md](02-add-a-building.md)) plus a `construction::Battery
{ charge: f32 }` component on the built entity, capped at `BATTERY_CAPACITY`.
`power::tick_power` is the only writer: surplus (`production > consumption`)
banks into every battery's `charge` up to its cap; a deficit drains it back
out — both capped per battery at `BATTERY_MAX_FLOW` units/sec so a big
swing ramps visibly instead of snapping a battery full/empty in one tick.
`PowerGrid::stored`/`capacity` are the summed totals across every battery,
and `powered_fraction` already accounts for discharge covering a deficit —
a second storage kind just needs its own component and a matching bank/
drain block in `tick_power`; nothing else needs to change to be "storage
aware", since `powered_fraction` is computed from the *coverage* achieved,
not from which kind supplied it.

If you want a new storage kind's *charge readout* to render (not just the
totals), `construction::sync_batteries` is the template: it swaps a child
mesh's material among 4 precomputed buckets (`GameAssets::
battery_strip_materials`, mixed empty→full like `cover::flora_material`'s
coverage overlay) rather than creating a material at runtime.

## Pitfalls

- **`tick_power` is the single writer.** Don't mutate `PowerGrid`, any
  `PowerOutput`, or any `Battery.charge` from anywhere else — every reader
  (info panel, debug tab, `sync_lightposts`, `sync_batteries`) assumes one
  consistent recompute per frame. Add a query/field to `tick_power` instead.
- **Ordering.** `sync_lightposts`/`sync_batteries` are pinned
  `.after(power::tick_power)` in `game.rs` because they read `PowerGrid`/
  `Battery` the same frame `tick_power` writes them — a plain same-tuple
  entry with no explicit ordering is not guaranteed to run after it. Any new
  reader that needs *this frame's* numbers (not next frame's) needs the same
  `.after(power::tick_power)`.
- **No wiring yet.** Every producer/consumer/battery is map-wide — a turbine
  on one side of the map can power a lightpost on the other. Circuits
  (RimWorld/SimCity-style local grids) are a deliberately postponed follow-up;
  don't half-implement adjacency checks for a single building without a plan
  for the whole grid.
- **Desire vs. demand.** `PowerConsumer::demand` is the *ceiling*; what
  actually gets summed into `PowerGrid::consumption` is `demand` scaled by
  how much the consumer currently *wants* power (the light post's
  `1.0 - daylight()`). Don't sum `demand` directly unless your consumer
  always wants full power.
- **Battery bucket art**: `sync_batteries` reuses the 4-bucket shared-material
  swap idiom (`cover::coverage_bucket`, `snow::snow_bucket`,
  `temperature::temperature_bucket`) — never call `materials.add(...)` per
  entity per frame; that's a runtime asset leak the whole codebase avoids.
