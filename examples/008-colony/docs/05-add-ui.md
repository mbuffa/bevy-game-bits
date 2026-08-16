# Add UI: buttons, panels, tooltip lines, debug tabs

All UI is plain Bevy `Node` UI, built imperatively in `ui::setup` (HUD) and
`debug_ui::setup` (debug window), styled entirely from `config.rs` constants
(`PANEL_BACKGROUND`, `PANEL_TEXT`, `BUTTON_*`, sizes). No egui.

The house pattern, everywhere: a **marker component** on the node, a
**spawn block** in setup, and small **sync/run systems** registered in the
UI group at the bottom of `game.rs` (ungated — UI stays live while paused).
Sync systems write state only on change (`if text.0 != wanted`) to avoid
churn.

## Recipe: selection-scoped action button (`TileAction`)

Square buttons next to the info panel, shown only when the selection
supports them. Example: `Cut`.

1. `TileAction` variant in `ui.rs` + entry in `TileAction::ALL` + arms in
   `label()` and `colors()` (color families: green = do-work, red =
   cancel/destroy, blue/amber = zone edits — consts in `config.rs`).
2. Availability arm in `update_actions`: the match decides `Display::Flex`
   vs `None` from what the selected entity has (`Has<ToCut>`, etc.).
3. Effect arm in `run_actions`: act on `selected.0` — insert a designation
   (`ToHarvest`, `ToCut`), arm a tool (`tool.0 = Some(Tool::…)`), or mark
   for a handler (`ToCancel`).

Setup spawns one button per `ALL` entry automatically — no setup changes.

Convention: **Cancel is generic.** It strips every cancellable designation
(`remove::<(ToHarvest, ToCut, ToDemolish)>()`) and marks blueprints
`ToCancel`. A new designation belongs in that remove-tuple, not behind its
own cancel button.

## Recipe: global tool button (`GeneralAction`)

The bottom-center bar; each button arms a `zones::Tool` for drag-placement.
Buttons are grouped into a RimWorld-style architect menu: one always-visible
`ActionCategory` button per `ActionCategory::ALL` entry (Zone, Structure,
Furniture, Power, Light, Roof, Cut, Demolish), each expanding a floating
sub-row of its `GeneralAction`s.

1. `GeneralAction` variant + `label()` arm, **and** an entry in the matching
   `ActionCategory::actions()` list — every `GeneralAction` must belong to
   exactly one category; nothing enforces this but the reviewer's eye, so
   double check the match in `ActionCategory::actions()`.
2. Arm in `run_general_actions` mapping it to a `Tool` variant. That system
   also collapses the menu (`open.0 = None`) after arming, so a fresh
   variant gets this for free.
3. The `Tool` itself: variant + `label()` (shown in the cursor tooltip while
   armed) + commit arm in `zones::drag_zone_tool` (guides 02/04). Right-click
   or Escape disarms; the tool stays armed across commits by design.

Adding a **new category** (rare): `ActionCategory` variant + entry in `ALL` +
`label()` arm + `actions()` arm. Setup spawns one column (sub-row + category
button) per `ALL` entry automatically — no setup changes.

The menu itself: `OpenCategory` resource (`Option<ActionCategory>`, `None` =
collapsed) drives visibility. `run_category_buttons` toggles it on press;
`sync_category_menu` follows it, setting each `CategorySubRow`'s
`Node.display` and tinting the open `CategoryButton`; `close_category_menu`
clears it on Escape/right-click, mirroring the tool-disarm in
`zones::drag_zone_tool`. Sub-rows are `position: Absolute` with `bottom:
Percent(100.0)` so they float above their own category button without
changing column height — every category button stays on one baseline
regardless of which sub-row (if any) is open.

## Recipe: a new panel

Template: the allowance panel (static frame + dynamic rows) or the clock
chip (simple text).

1. Marker component (e.g. `AllowancePanel`) + spawn block in `ui::setup`:
   absolute-positioned root `Node`, `BackgroundColor(PANEL_BACKGROUND)`,
   `PANEL_MARGIN` from the edges.
2. **Click-through guard**: give the root `Interaction::default()` if the
   panel overlaps the map — `click_select`, `click_move`, `drag_zone_tool`,
   and `update_tooltip` all skip clicks/hover while any `Interaction` node
   is hovered. Purely-informational chips that must never eat clicks (pause
   banner, clock) deliberately omit it.
3. Sync system(s) keeping content fresh; register in the UI tuple in
   `game.rs`. For per-entity rows, pair an `Added<X>`-driven spawn system
   with a cleanup check (`spawn_allowance_rows` / the row-despawn in
   `sync_allowance_buttons`).
4. Show/hide via `Visibility` (whole panel) or `Node.display` (rows) — see
   `sync_pause_banner` vs `sync_grow_checkbox` (zone selected) /
   `sync_manual_checkbox` (pawn selected, toggles `units::ManualMode`) /
   `sync_history_panel` (pawn selected *and* a separate open/closed
   `HistoryOpen` resource — two conditions ANDed together).

For expandable sections (header rows toggling detail rows), see the
left-center stock panel: `StockCategoryRow` presses flip a `HashSet` in the
`StockUiState` resource, `sync_stock_panel` follows it with `Node.display`.
Its vertical centering trick — an absolute `top: 0 / bottom: 0` wrapper
column with `justify_content: Center` — must NOT carry `Interaction`, or
the invisible full-height strip would trip the click-through guard.

Checkbox convention: checked state is shape-encoded with the `CHECKBOX_MARK`
glyph (`CheckGlyph` child visibility), fill color only a secondary cue —
colorblind-safe. Reuse it for any new toggle.

## Recipe: a scrollable panel

Template: the pawn History panel (`ui::HistoryPanel`) — the only scrollable
UI in the project so far. Needed once a panel's content (a growing list) no
longer fits a fixed footprint.

1. Fixed-size root `Node` with `overflow: Overflow::scroll_y()` (or
   `scroll_x`/`scroll()`) plus a `ScrollPosition` component. A single child
   (e.g. one multi-line `Text`, like `HistoryList`) is enough — the parent
   clips it to its own width/height and `ScrollPosition` shifts it inside
   that clipped viewport. Bevy re-clamps `ScrollPosition` itself whenever
   layout makes the current offset invalid, so a scroll system only needs to
   clamp the *lower* bound (`.max(0.0)`); it never needs to compute content
   height to clamp the upper end.
2. `Interaction::default()` on the root doubles as the usual click-through
   guard **and** the scroll system's own hover check — see step 3.
3. A `scroll_*` system reading `Res<AccumulatedMouseScroll>` (the same
   resource `scene::zoom_camera` uses), gated on `*interaction !=
   Interaction::None` for that panel's root, nudges `ScrollPosition`'s `y`
   (or `x`) by `-dy * <a speed const>`.
4. **Camera-zoom conflict**: `scene::zoom_camera` reads the same
   `AccumulatedMouseScroll` unconditionally. Any scrollable panel must add a
   `Query<&Interaction, With<YourPanelMarker>>` guard there (see
   `zoom_camera`'s `history_panel` param) so scrolling the panel doesn't also
   zoom the map underneath.
5. If the panel is nested inside a flex row (like `HistoryPanel`, a third
   child of `SelectionHud` next to the info panel and action-button column)
   rather than its own absolute-positioned root, give it a `margin` pulling
   it clear of anything else anchored to the same screen edge — e.g. the
   bottom-center general Actions bar, which spans the full window width and
   can otherwise poke through a tall bottom-anchored sibling's lower edge.

## UI layering

Panels are separate UI-tree roots with no `ZIndex` by default, so stacking
follows spawn order unless overridden. HUD panels implicitly sit at
`Z_HUD` (0, `config.rs`); the devtools window carries an explicit
`GlobalZIndex(Z_DEVTOOLS)` so it always renders above the HUD, regardless of
spawn order. A new panel that must float above the HUD should take an
explicit `GlobalZIndex(Z_*)` from `config.rs` rather than relying on where its
spawn call happens to sit in `game.rs`'s `Startup` chain.

## Recipe: a tooltip line

`ui::update_tooltip` builds the hover string from map state (`TerrainMap`,
`CoverMap`, `HumidityMap`, `TemperatureMap`, `RoofMap`). Append your clause
where the label is assembled; keep it short — it's a cursor tooltip. New
per-cell state should come from a resource lookup, not an entity query scan
(the Room-insulation clause is the one exception: rooms are few, so the
`(&ZoneRegion, &Room)` scan stays cheap). The system sits at the 16-param
limit — new reads join one of the bundled tuple params (`air`,
`plant_queries`).

## Recipe: a debug tab

Dev switches and live internals go in the backtick window, not the HUD.
Example: the Weather tab (rain toggle).

1. `DebugTab` variant in `debug_ui.rs` + entry in `DebugTab::ALL` +
   `label()` arm — the tab bar builds itself from `ALL`.
2. A content column in `debug_ui::setup` tagged
   `DebugTabContent(DebugTab::X)`; `sync_tabs` handles showing/hiding.
3. Systems: a `run_*` for interactions and a `sync_*`/`update_*` keeping the
   content fresh; register them in the debug tuple in `game.rs`. Skip work
   while hidden the way `update_jobs_tab` early-outs on
   `Visibility::Hidden`.

Dev toggles flip a plain resource (`Weather.rain`) and let the normal
systems react — the debug UI should never reach into gameplay directly.

## Pitfalls

- Bevy's default font is glyph-poor: no em-dash (use `-`), no ✓ (that's why
  `CHECKBOX_MARK` is `×` from the bundled FiraSans via
  `CHECKBOX_MARK_FONT`). Test any non-ASCII label in-game.
- UI systems run every frame; guard text/color writes behind an inequality
  check, and rebuild collections only on `Changed<…>`/`Added<…>` where
  possible.
- Buttons need the `Button` component for `Interaction` to fire, and
  interaction handlers should filter `Changed<Interaction>` +
  `Interaction::Pressed`.
- World-anchored UI (pawn labels) converts via `camera.world_to_viewport`
  each frame (`sync_pawn_labels`) and must handle the off-screen `Err` by
  hiding.
