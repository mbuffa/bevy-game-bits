use std::collections::HashSet;

use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::prelude::*;
use bevy_ecs_ldtk::prelude::GridCoords;

use crate::config::*;
use crate::construction::{
    Battery, BedOrientation, Blueprint, ConstructionMap, Cooler, Footprint, ToCancel, ToDemolish,
};
use crate::cover::{self, CoverMap};
use crate::crops::Crop;
use crate::daynight::{GameClock, GameSpeed, SkyWidgetImage};
use crate::director::{Objective, PawnStatus};
use crate::flora::{BerryBush, Harvestable, ToHarvest};
use crate::game::SimState;
use crate::history::JobHistory;
use crate::items::{self, ItemKind, ItemStack, StockCategory};
use crate::map::{self, RoofMap, RoofPolicy, TerrainMap};
use crate::needs::Needs;
use crate::power::PowerOutput;
use crate::scene::{self, MainCamera};
use crate::selection::{cursor_to_cell, Selectable, SelectedEntity};
use crate::shrubs::Shrub;
use crate::temperature;
use crate::trees::{ToCut, Tree};
use crate::units::{Allowance, ManualMode, Pawn, Skills, Tier};
use crate::weather::{HumidityMap, WindBand, WindExposureMap};
use crate::zones::{self, ActiveTool, GrowOrder, Tool, Zone, ZonePlant, ZoneRegion};

/// Screen-space selection square: translucent fill with white corner
/// brackets, repositioned every frame over the selected entity.
#[derive(Component)]
pub struct SelectionIndicator;

#[derive(Component)]
pub struct InfoPanel;

#[derive(Component)]
pub struct InfoName;

#[derive(Component)]
pub struct InfoDetail;

/// Bottom-left selection HUD: the info panel plus the tile-action column,
/// shown while something is selected.
#[derive(Component)]
pub struct SelectionHud;

/// Player orders on the selected tile/entity, shown as a column of square
/// buttons next to the info panel (RimWorld-style). Selection-scoped, unlike
/// `GeneralAction`s which are available with nothing selected.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TileAction {
    /// Queue the selected bush for harvesting now (`ToHarvest`).
    Harvest,
    /// Queue the selected tree for felling (`ToCut`, growth >= 50%).
    Cut,
    /// Queue the selected built structure for demolition (`ToDemolish`).
    Demolish,
    /// Revoke every cancellable designation (today: `ToHarvest`, `ToCut`,
    /// `ToDemolish`).
    Cancel,
    /// Remove the selected zone entirely.
    Delete,
    /// Arm the drag tool to grow the selected zone (bounding box of old
    /// union drawn).
    Expand,
    /// Arm the drag tool to clip the selected zone (old intersect drawn).
    Shrink,
}

impl TileAction {
    pub const ALL: [TileAction; 7] = [
        TileAction::Harvest,
        TileAction::Cut,
        TileAction::Demolish,
        TileAction::Cancel,
        TileAction::Delete,
        TileAction::Expand,
        TileAction::Shrink,
    ];

    fn label(&self) -> &'static str {
        match self {
            TileAction::Harvest => "Harvest",
            TileAction::Cut => "Cut",
            TileAction::Demolish => "Demolish",
            TileAction::Cancel => "Cancel",
            TileAction::Delete => "Delete",
            TileAction::Expand => "Expand",
            TileAction::Shrink => "Shrink",
        }
    }

    /// (base, hover, pressed) background colors.
    fn colors(&self) -> (Color, Color, Color) {
        match self {
            TileAction::Harvest | TileAction::Cut | TileAction::Demolish => {
                (BUTTON_BACKGROUND, BUTTON_HOVER, BUTTON_PRESSED)
            }
            TileAction::Cancel | TileAction::Delete => {
                (CANCEL_BACKGROUND, CANCEL_HOVER, CANCEL_PRESSED)
            }
            TileAction::Expand => (EXPAND_BACKGROUND, EXPAND_HOVER, EXPAND_PRESSED),
            TileAction::Shrink => (SHRINK_BACKGROUND, SHRINK_HOVER, SHRINK_PRESSED),
        }
    }
}

/// A square button running one `TileAction` on the selection.
#[derive(Component)]
pub struct ActionButton {
    pub action: TileAction,
}

/// Global orders available with nothing selected, shown on an always-visible
/// bottom-center bar (as opposed to selection-scoped `TileAction`s).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GeneralAction {
    /// Arm the drag tool to draw a new stockpile zone.
    CreateStockpile,
    /// Arm the drag tool to draw a new growing zone.
    CreateGrowing,
    /// Arm the drag tool to fill cells with wall/door/turbine blueprints.
    CreateWall,
    CreateDoor,
    /// Arm the drag tool to tile a snapped grid of 2x1 bed blueprints
    /// (`zones::Tool::PlaceBed`), horizontal by default — R rotates while
    /// armed.
    CreateBed,
    CreateTurbine,
    /// Arm the drag tool to tile a snapped grid of 2x2 solar-panel
    /// blueprints (`zones::Tool::PlaceSolarPanel`).
    CreateSolarPanel,
    /// Arm the drag tool to fill cells with lightpost blueprints
    /// (`zones::Tool::PlaceLightpost`).
    CreateLightpost,
    /// Arm the drag tool to fill cells with battery blueprints
    /// (`zones::Tool::PlaceBattery`).
    CreateBattery,
    /// Arm the drag tool to fill cells with cooler blueprints
    /// (`zones::Tool::PlaceCooler`) — a wall-segment climate unit.
    CreateCooler,
    /// Arm the roof-policy stamps (paint orders on `map::RoofMap`, not
    /// zones): plan roofs, forbid them, or erase the intent entirely.
    RoofArea,
    NoRoofArea,
    ClearRoofArea,
    /// Arm the drag tool that marks every cuttable tree in the rectangle
    /// with `ToCut` (`zones::Tool::DesignateCut`); saplings are skipped.
    CutArea,
    /// Arm the drag tool that removes `ToCut` from every marked tree in the
    /// rectangle (`zones::Tool::CancelCut`) — the area counterpart of the
    /// per-tile Cancel button.
    CancelCutArea,
    /// Arm the drag tool that marks every built structure in the rectangle
    /// with `ToDemolish` (`zones::Tool::DesignateDemolish`).
    DemolishArea,
    /// Arm the drag tool that removes `ToDemolish` from every marked
    /// structure in the rectangle (`zones::Tool::CancelDemolish`) — the area
    /// counterpart of the per-tile Cancel button.
    CancelDemolishArea,
}

impl GeneralAction {
    // Iteration order comes from `ActionCategory::ALL` + `actions()` now
    // (the architect-menu categories), not a flat `ALL` here.

    fn label(&self) -> &'static str {
        match self {
            GeneralAction::CreateStockpile => "Stockpile",
            GeneralAction::CreateGrowing => "Growing",
            GeneralAction::CreateWall => "Wall",
            GeneralAction::CreateDoor => "Door",
            GeneralAction::CreateBed => "Bed",
            GeneralAction::CreateTurbine => "Turbine",
            GeneralAction::CreateSolarPanel => "Solar panel",
            GeneralAction::CreateLightpost => "Lightpost",
            GeneralAction::CreateBattery => "Battery",
            GeneralAction::CreateCooler => "Cooler",
            GeneralAction::RoofArea => "Roof",
            GeneralAction::NoRoofArea => "No roof",
            GeneralAction::ClearRoofArea => "Clear roof",
            GeneralAction::CutArea => "Cut",
            GeneralAction::CancelCutArea => "Cancel cut",
            GeneralAction::DemolishArea => "Demolish",
            GeneralAction::CancelDemolishArea => "Cancel demolish",
        }
    }
}

/// A square button running one `GeneralAction`.
#[derive(Component)]
pub struct GeneralActionButton {
    pub action: GeneralAction,
}

/// Groups the Actions bar into a RimWorld-style architect menu: a row of
/// always-visible category buttons, each expanding a floating sub-row of its
/// `GeneralAction`s. Every `GeneralAction` must appear in exactly one
/// category's `actions()` list — the compiler won't catch a variant left out.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ActionCategory {
    Zone,
    Structure,
    Furniture,
    Power,
    Light,
    Roof,
    Cut,
    Demolish,
}

impl ActionCategory {
    pub const ALL: [ActionCategory; 8] = [
        ActionCategory::Zone,
        ActionCategory::Structure,
        ActionCategory::Furniture,
        ActionCategory::Power,
        ActionCategory::Light,
        ActionCategory::Roof,
        ActionCategory::Cut,
        ActionCategory::Demolish,
    ];

    fn label(&self) -> &'static str {
        match self {
            ActionCategory::Zone => "Zone",
            ActionCategory::Structure => "Structure",
            ActionCategory::Furniture => "Furniture",
            ActionCategory::Power => "Power",
            ActionCategory::Light => "Light",
            ActionCategory::Roof => "Roof",
            ActionCategory::Cut => "Cut",
            ActionCategory::Demolish => "Demolish",
        }
    }

    fn actions(&self) -> &'static [GeneralAction] {
        match self {
            ActionCategory::Zone => &[GeneralAction::CreateStockpile, GeneralAction::CreateGrowing],
            ActionCategory::Structure => &[
                GeneralAction::CreateWall,
                GeneralAction::CreateDoor,
                GeneralAction::CreateCooler,
            ],
            ActionCategory::Furniture => &[GeneralAction::CreateBed],
            ActionCategory::Power => &[
                GeneralAction::CreateTurbine,
                GeneralAction::CreateSolarPanel,
                GeneralAction::CreateBattery,
            ],
            ActionCategory::Light => &[GeneralAction::CreateLightpost],
            ActionCategory::Roof => &[
                GeneralAction::RoofArea,
                GeneralAction::NoRoofArea,
                GeneralAction::ClearRoofArea,
            ],
            ActionCategory::Cut => &[GeneralAction::CutArea, GeneralAction::CancelCutArea],
            ActionCategory::Demolish => &[
                GeneralAction::DemolishArea,
                GeneralAction::CancelDemolishArea,
            ],
        }
    }
}

/// The always-visible button opening/closing one `ActionCategory`'s sub-row.
#[derive(Component)]
pub struct CategoryButton {
    pub category: ActionCategory,
}

/// The floating row of `GeneralActionButton`s for one category, shown only
/// while `OpenCategory` matches it.
#[derive(Component)]
pub struct CategorySubRow {
    pub category: ActionCategory,
}

/// Which `ActionCategory`'s sub-row is currently expanded on the bottom-center
/// Actions bar, if any.
#[derive(Resource, Default)]
pub struct OpenCategory(pub Option<ActionCategory>);

/// Right-side debug panel listing every pawn and its current activity.
#[derive(Component)]
pub struct JobPanelText;

/// Floating name tag following its pawn on screen.
#[derive(Component)]
pub struct PawnLabel(Entity);

/// One of the four world-anchored N/E/S/W map labels. East = world +X
/// (`daynight::sky_position`'s sunrise direction), north = world -Z (the
/// `grid.y + 1` neighbor — see `compass_anchor`).
#[derive(Clone, Copy)]
pub enum CompassDir {
    North,
    East,
    South,
    West,
}

/// Marker on a compass letter's floating label, tracking which direction it
/// represents so `sync_compass_labels` can re-derive its world anchor.
#[derive(Component)]
pub struct CompassLabel(CompassDir);

/// Top-right panel with per-pawn work-type toggles.
#[derive(Component)]
pub struct AllowancePanel;

/// One panel row per pawn (back-pointer for cleanup).
#[derive(Component)]
pub struct AllowanceRow(Entity);

#[derive(Clone, Copy)]
pub enum ToggleWork {
    Harvest,
    Haul,
    Farming,
    Forestry,
    Build,
}

impl ToggleWork {
    /// Column order of the allowance grid; headers and checkbox rows both
    /// iterate this, so new work types only need a new entry here.
    pub const ALL: [(ToggleWork, &'static str); 5] = [
        (ToggleWork::Harvest, "Harvest"),
        (ToggleWork::Haul, "Haul"),
        (ToggleWork::Farming, "Farm"),
        (ToggleWork::Forestry, "Cut"),
        (ToggleWork::Build, "Build"),
    ];
}

/// A button flipping one work type on one pawn's `Allowance`.
#[derive(Component)]
pub struct AllowanceToggle {
    pub pawn: Entity,
    pub work: ToggleWork,
}

/// The mark glyph inside a checkbox; shown while the work type is allowed.
#[derive(Component)]
pub struct CheckGlyph;

/// Container for the Grow checkbox row in the info panel; its `Display` is
/// toggled to show only while a growing zone is selected.
#[derive(Component)]
pub struct GrowRow;

/// The Grow checkbox button; a press flips the selected zone's `GrowOrder`.
#[derive(Component)]
pub struct GrowCheckbox;

/// Container for the Sleep-need gauge row in the info panel; its `Display`
/// is toggled to show only while a pawn is selected (see
/// `sync_needs_panel`). The only need today — more (Food, Recreation, ...)
/// would each get their own row here, driven by `needs::Needs::gauges`.
#[derive(Component)]
pub struct NeedsSection;

/// The percent-width fill bar inside the Sleep gauge track.
#[derive(Component)]
pub struct SleepGaugeFill;

/// The numeric readout next to the Sleep gauge (e.g. "72").
#[derive(Component)]
pub struct SleepGaugeValue;

/// Container for the Manual-mode checkbox row in the info panel; its
/// `Display` is toggled to show only while a pawn is selected.
#[derive(Component)]
pub struct ManualRow;

/// The Manual checkbox button; a press drafts/undrafts the selected pawn.
#[derive(Component)]
pub struct ManualCheckbox;

/// Container for the cooler target-temperature row in the info panel; its
/// `Display` is toggled to show only while a `Cooler` is selected (see
/// `sync_target_temp_row`).
#[derive(Component)]
pub struct TargetTempRow;

/// One of the row's +/- nudgers; a press steps the selected cooler's
/// `target_c` by `TARGET_TEMP_STEP`, clamped to `TARGET_TEMP_MIN..=MAX`
/// (`run_target_temp_buttons`) — the same devtool nudger pattern as
/// `debug_ui::WaterLevelButton`/`HourButton`.
#[derive(Component)]
pub struct TargetTempButton {
    pub raise: bool,
}

/// The row's numeric readout (e.g. "18°C").
#[derive(Component)]
pub struct TargetTempValue;

/// A tab on the pawn info panel: Job History, Skills, Needs. Each opens a
/// richer view of data the info panel/inline rows only summarize. New tab =
/// a variant here, an entry in `ALL`, a `label()` arm, a `PawnTabContent`
/// column in `setup`, and a system keeping that content fresh — the same
/// recipe as `debug_ui::DebugTab`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PawnTab {
    JobHistory,
    Skills,
    Needs,
}

impl PawnTab {
    /// Tab-bar order; the bar builds itself from this.
    pub const ALL: [PawnTab; 3] = [PawnTab::JobHistory, PawnTab::Skills, PawnTab::Needs];

    fn label(&self) -> &'static str {
        match self {
            PawnTab::JobHistory => "Job History",
            PawnTab::Skills => "Skills",
            PawnTab::Needs => "Needs",
        }
    }
}

/// Which pawn tab's window is open, if any. Unlike `debug_ui::ActiveDebugTab`
/// this can be closed entirely: pressing the already-active tab clears it
/// back to `None` (see `run_pawn_tab_buttons`).
#[derive(Resource, Default)]
pub struct ActivePawnTab(pub Option<PawnTab>);

/// Container for the tab bar (Job History / Skills / Needs) at the top of
/// the info panel; its `Display` is toggled to show only while a pawn is
/// selected (see `sync_pawn_tabs`).
#[derive(Component)]
pub struct PawnTabBar;

/// A tab-bar button; a press opens its tab (or closes the window if it was
/// already the active tab).
#[derive(Component)]
pub struct PawnTabButton(PawnTab);

/// Center-left window showing the active tab's content, shown only while a
/// pawn is selected and `ActivePawnTab` is `Some` (see `sync_pawn_tabs`). At
/// the same left-center slot as the Stock panel above it in `setup`; they're
/// allowed to overlap; a pawn's tab window and the always-on Stock panel are
/// rarely both wanted at once.
#[derive(Component)]
pub struct PawnTabWindow;

/// A tab's content column inside `PawnTabWindow`, shown only while its tab
/// is active.
#[derive(Component)]
pub struct PawnTabContent(PawnTab);

/// The single multi-line text listing the selected pawn's history, newest
/// entry first (see `history::JobHistory`), inside the Job History tab.
#[derive(Component)]
pub struct HistoryList;

/// Which `units::Skills` field a `SkillValueText` label displays.
#[derive(Clone, Copy)]
pub enum SkillKind {
    Harvest,
    Haul,
    Farm,
    Forestry,
    Build,
}

impl SkillKind {
    const ALL: [SkillKind; 5] = [
        SkillKind::Harvest,
        SkillKind::Haul,
        SkillKind::Farm,
        SkillKind::Forestry,
        SkillKind::Build,
    ];

    fn label(&self) -> &'static str {
        match self {
            SkillKind::Harvest => "Harvest",
            SkillKind::Haul => "Haul",
            SkillKind::Farm => "Farm",
            SkillKind::Forestry => "Forestry",
            SkillKind::Build => "Build",
        }
    }

    fn tier(&self, skills: &Skills) -> Tier {
        match self {
            SkillKind::Harvest => skills.harvest,
            SkillKind::Haul => skills.haul,
            SkillKind::Farm => skills.farm,
            SkillKind::Forestry => skills.forestry,
            SkillKind::Build => skills.build,
        }
    }
}

/// A skill row's tier readout in the Skills tab.
#[derive(Component)]
pub struct SkillValueText {
    skill: SkillKind,
}

/// A need gauge's fill bar in the Needs tab, indexed to match
/// `Needs::gauges()`'s iteration order.
#[derive(Component)]
pub struct NeedsTabGaugeFill {
    index: usize,
}

/// A need gauge's numeric readout in the Needs tab, indexed to match
/// `Needs::gauges()`'s iteration order.
#[derive(Component)]
pub struct NeedsTabGaugeValue {
    index: usize,
}

/// Container for the plant-selector row in the info panel; like `GrowRow`,
/// shown only while a growing zone is selected.
#[derive(Component)]
pub struct PlantRow;

/// The plant cycle button; a press steps the selected zone's `ZonePlant`
/// (Wheat -> Trees -> Wheat).
#[derive(Component)]
pub struct PlantButton;

/// The label inside the plant button, kept at the zone's current choice.
#[derive(Component)]
pub struct PlantButtonText;

/// Small panel following the cursor with info on the hovered tile. Shows
/// the terrain type for now; meant to grow richer over time.
#[derive(Component)]
pub struct TooltipPanel;

#[derive(Component)]
pub struct TooltipText;

/// Top-center "Paused" chip, shown while `SimState::Paused`.
#[derive(Component)]
pub struct PauseBanner;

/// Top-center pawn selector bar; chips are added as pawns spawn.
#[derive(Component)]
pub struct PortraitBar;

/// One selector chip; clicking selects its pawn, clicking again (already
/// selected) centers the camera on it.
#[derive(Component)]
pub struct PortraitButton(Entity);

/// Top-left "Day 2 — 14:30" chip, driven by the `GameClock`.
#[derive(Component)]
pub struct ClockLabel;

/// Game-speed selector row under the clock chip: one button per multiplier,
/// pressing it sets `GameSpeed` and resumes if paused.
#[derive(Component)]
pub struct SpeedButton(pub f32);

/// The selector row's Pause button, sharing `SimState` with the Space bar.
#[derive(Component)]
pub struct SpeedPauseButton;

/// A stock-panel category row (Raw Food, Materials, ...); a click toggles
/// its per-kind detail rows.
#[derive(Component)]
pub struct StockCategoryRow {
    pub category: StockCategory,
}

/// The amount text on a stock category row.
#[derive(Component)]
pub struct StockCategoryAmount {
    pub category: StockCategory,
}

/// A per-kind detail row on the stock panel, shown while its category is
/// expanded.
#[derive(Component)]
pub struct StockDetailRow {
    pub kind: ItemKind,
}

/// The amount text on a stock detail row.
#[derive(Component)]
pub struct StockDetailAmount {
    pub kind: ItemKind,
}

/// Which stock-panel categories are expanded into per-kind rows.
#[derive(Resource, Default)]
pub struct StockUiState {
    pub expanded: HashSet<StockCategory>,
}

pub fn setup(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    sky_image: Res<SkyWidgetImage>,
) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
            BackgroundColor(SELECTION_FILL),
            Visibility::Hidden,
            SelectionIndicator,
        ))
        .with_children(|parent| {
            // One L-shaped bracket per corner, drawn with borders on the two
            // outer sides only.
            let corners = [
                (Val::Px(0.0), Val::Auto, Val::Px(0.0), Val::Auto), // top-left
                (Val::Px(0.0), Val::Auto, Val::Auto, Val::Px(0.0)), // top-right
                (Val::Auto, Val::Px(0.0), Val::Px(0.0), Val::Auto), // bottom-left
                (Val::Auto, Val::Px(0.0), Val::Auto, Val::Px(0.0)), // bottom-right
            ];
            for (top, bottom, left, right) in corners {
                parent.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        top,
                        bottom,
                        left,
                        right,
                        width: Val::Px(BRACKET_ARM),
                        height: Val::Px(BRACKET_ARM),
                        border: UiRect {
                            top: if top == Val::Auto {
                                Val::ZERO
                            } else {
                                Val::Px(BRACKET_THICKNESS)
                            },
                            bottom: if bottom == Val::Auto {
                                Val::ZERO
                            } else {
                                Val::Px(BRACKET_THICKNESS)
                            },
                            left: if left == Val::Auto {
                                Val::ZERO
                            } else {
                                Val::Px(BRACKET_THICKNESS)
                            },
                            right: if right == Val::Auto {
                                Val::ZERO
                            } else {
                                Val::Px(BRACKET_THICKNESS)
                            },
                        },
                        ..default()
                    },
                    BorderColor::all(SELECTION_BRACKET),
                ));
            }
        });

    // Bottom-left selection HUD: info panel + tile-action column.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(PANEL_MARGIN),
                bottom: Val::Px(PANEL_MARGIN),
                column_gap: Val::Px(8.0),
                align_items: AlignItems::FlexEnd,
                ..default()
            },
            Visibility::Hidden,
            // Interaction makes the HUD a hover target, so `click_select`
            // can ignore clicks landing on it.
            Interaction::default(),
            SelectionHud,
        ))
        .with_children(|hud| {
            hud.spawn((
                Node {
                    padding: UiRect::axes(Val::Px(12.0), Val::Px(8.0)),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(2.0),
                    ..default()
                },
                BackgroundColor(PANEL_BACKGROUND),
                InfoPanel,
            ))
            .with_children(|panel| {
                // Tab bar: Job History / Skills / Needs. Hidden except while
                // a pawn is selected (see `sync_pawn_tabs`); a press opens
                // the matching content in `PawnTabWindow` (or closes it, if
                // that tab was already active).
                panel
                    .spawn((
                        Node {
                            display: Display::None,
                            column_gap: Val::Px(4.0),
                            margin: UiRect::bottom(Val::Px(4.0)),
                            ..default()
                        },
                        PawnTabBar,
                    ))
                    .with_children(|tabs| {
                        for tab in PawnTab::ALL {
                            tabs.spawn((
                                Button,
                                PawnTabButton(tab),
                                Node {
                                    padding: UiRect::axes(Val::Px(8.0), Val::Px(2.0)),
                                    ..default()
                                },
                                BackgroundColor(BUTTON_BACKGROUND),
                            ))
                            .with_child((
                                Text::new(tab.label()),
                                TextFont {
                                    font_size: 11.0,
                                    ..default()
                                },
                                TextColor(PANEL_TEXT),
                            ));
                        }
                    });
                panel.spawn((
                    Text::new(""),
                    TextFont {
                        font_size: 16.0,
                        ..default()
                    },
                    TextColor(PANEL_TEXT),
                    InfoName,
                ));
                panel.spawn((
                    Text::new(""),
                    TextFont {
                        font_size: 13.0,
                        ..default()
                    },
                    TextColor(PANEL_TEXT),
                    InfoDetail,
                ));
                // Sleep-need gauge; hidden except while a pawn is selected
                // (see `sync_needs_panel`).
                panel
                    .spawn((
                        Node {
                            display: Display::None,
                            align_items: AlignItems::Center,
                            column_gap: Val::Px(6.0),
                            margin: UiRect::top(Val::Px(4.0)),
                            ..default()
                        },
                        NeedsSection,
                    ))
                    .with_children(|row| {
                        row.spawn((
                            Text::new("Sleep"),
                            TextFont {
                                font_size: 13.0,
                                ..default()
                            },
                            TextColor(PANEL_TEXT),
                        ));
                        row.spawn((
                            Node {
                                width: Val::Px(GAUGE_WIDTH),
                                height: Val::Px(GAUGE_HEIGHT),
                                ..default()
                            },
                            BackgroundColor(GAUGE_TRACK_COLOR),
                        ))
                        .with_children(|track| {
                            track.spawn((
                                Node {
                                    width: Val::Percent(100.0),
                                    height: Val::Percent(100.0),
                                    ..default()
                                },
                                BackgroundColor(GAUGE_FILL_HIGH_COLOR),
                                SleepGaugeFill,
                            ));
                        });
                        row.spawn((
                            Text::new(""),
                            TextFont {
                                font_size: 13.0,
                                ..default()
                            },
                            TextColor(PANEL_TEXT),
                            SleepGaugeValue,
                        ));
                    });
                // Grow order checkbox; hidden except while a growing zone is
                // selected (see `sync_grow_checkbox`).
                panel
                    .spawn((
                        Node {
                            display: Display::None,
                            align_items: AlignItems::Center,
                            column_gap: Val::Px(6.0),
                            margin: UiRect::top(Val::Px(4.0)),
                            ..default()
                        },
                        GrowRow,
                    ))
                    .with_children(|row| {
                        row.spawn((
                            Button,
                            GrowCheckbox,
                            Node {
                                width: Val::Px(CHECKBOX_SIZE),
                                height: Val::Px(CHECKBOX_SIZE),
                                border: UiRect::all(Val::Px(1.0)),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                ..default()
                            },
                            BorderColor::all(CHECKBOX_BORDER),
                            BackgroundColor(CHECKBOX_ON_COLOR),
                        ))
                        .with_child((
                            Text::new(CHECKBOX_MARK),
                            TextFont {
                                font: asset_server.load(CHECKBOX_MARK_FONT),
                                font_size: 14.0,
                                ..default()
                            },
                            TextColor(PANEL_TEXT),
                            CheckGlyph,
                        ));
                        row.spawn((
                            Text::new("Grow"),
                            TextFont {
                                font_size: 13.0,
                                ..default()
                            },
                            TextColor(PANEL_TEXT),
                        ));
                    });
                // Manual-mode (draft) checkbox; hidden except while a pawn
                // is selected (see `sync_manual_checkbox`).
                panel
                    .spawn((
                        Node {
                            display: Display::None,
                            align_items: AlignItems::Center,
                            column_gap: Val::Px(6.0),
                            margin: UiRect::top(Val::Px(4.0)),
                            ..default()
                        },
                        ManualRow,
                    ))
                    .with_children(|row| {
                        row.spawn((
                            Button,
                            ManualCheckbox,
                            Node {
                                width: Val::Px(CHECKBOX_SIZE),
                                height: Val::Px(CHECKBOX_SIZE),
                                border: UiRect::all(Val::Px(1.0)),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                ..default()
                            },
                            BorderColor::all(CHECKBOX_BORDER),
                            BackgroundColor(CHECKBOX_OFF_COLOR),
                        ))
                        .with_child((
                            Text::new(CHECKBOX_MARK),
                            TextFont {
                                font: asset_server.load(CHECKBOX_MARK_FONT),
                                font_size: 14.0,
                                ..default()
                            },
                            TextColor(PANEL_TEXT),
                            CheckGlyph,
                        ));
                        row.spawn((
                            Text::new("Manual"),
                            TextFont {
                                font_size: 13.0,
                                ..default()
                            },
                            TextColor(PANEL_TEXT),
                        ));
                    });
                // Cooler target-temperature nudger; hidden except while a
                // cooler is selected (see `sync_target_temp_row`). Same
                // +/- nudger pattern as `debug_ui`'s water-level/hour
                // buttons, laid out as one compact row instead of a column.
                panel
                    .spawn((
                        Node {
                            display: Display::None,
                            align_items: AlignItems::Center,
                            column_gap: Val::Px(6.0),
                            margin: UiRect::top(Val::Px(4.0)),
                            ..default()
                        },
                        TargetTempRow,
                    ))
                    .with_children(|row| {
                        row.spawn((
                            Text::new("Target"),
                            TextFont {
                                font_size: 13.0,
                                ..default()
                            },
                            TextColor(PANEL_TEXT),
                        ));
                        row.spawn((
                            Button,
                            TargetTempButton { raise: false },
                            Node {
                                padding: UiRect::axes(Val::Px(8.0), Val::Px(2.0)),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                ..default()
                            },
                            BackgroundColor(BUTTON_BACKGROUND),
                        ))
                        .with_child((
                            Text::new("-"),
                            TextFont {
                                font_size: 13.0,
                                ..default()
                            },
                            TextColor(PANEL_TEXT),
                        ));
                        row.spawn((
                            Text::new(""),
                            TextFont {
                                font_size: 13.0,
                                ..default()
                            },
                            TextColor(PANEL_TEXT),
                            TargetTempValue,
                        ));
                        row.spawn((
                            Button,
                            TargetTempButton { raise: true },
                            Node {
                                padding: UiRect::axes(Val::Px(8.0), Val::Px(2.0)),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                ..default()
                            },
                            BackgroundColor(BUTTON_BACKGROUND),
                        ))
                        .with_child((
                            Text::new("+"),
                            TextFont {
                                font_size: 13.0,
                                ..default()
                            },
                            TextColor(PANEL_TEXT),
                        ));
                    });
                // Plant selector; hidden except while a growing zone is
                // selected (see `sync_plant_button`).
                panel
                    .spawn((
                        Node {
                            display: Display::None,
                            align_items: AlignItems::Center,
                            column_gap: Val::Px(6.0),
                            margin: UiRect::top(Val::Px(4.0)),
                            ..default()
                        },
                        PlantRow,
                    ))
                    .with_children(|row| {
                        row.spawn((
                            Text::new("Plant:"),
                            TextFont {
                                font_size: 13.0,
                                ..default()
                            },
                            TextColor(PANEL_TEXT),
                        ));
                        row.spawn((
                            Button,
                            PlantButton,
                            Node {
                                padding: UiRect::axes(Val::Px(8.0), Val::Px(2.0)),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                border: UiRect::all(Val::Px(1.0)),
                                ..default()
                            },
                            BorderColor::all(CHECKBOX_BORDER),
                            BackgroundColor(BUTTON_BACKGROUND),
                        ))
                        .with_child((
                            Text::new(""),
                            TextFont {
                                font_size: 12.0,
                                ..default()
                            },
                            TextColor(PANEL_TEXT),
                            PlantButtonText,
                        ));
                    });
            });
            // One square button per possible tile action; `update_actions`
            // shows the ones the selection supports.
            hud.spawn(Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(6.0),
                ..default()
            })
            .with_children(|column| {
                for action in TileAction::ALL {
                    column
                        .spawn((
                            Button,
                            ActionButton { action },
                            Node {
                                display: Display::None,
                                width: Val::Px(ACTION_BUTTON_SIZE),
                                height: Val::Px(ACTION_BUTTON_SIZE),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                ..default()
                            },
                            BackgroundColor(action.colors().0),
                        ))
                        .with_child((
                            Text::new(action.label()),
                            TextFont {
                                font_size: 11.0,
                                ..default()
                            },
                            TextColor(PANEL_TEXT),
                        ));
                }
            });
        });

    // Bottom-center general Actions bar: global orders, available with
    // nothing selected, grouped into a RimWorld-style architect menu — one
    // always-visible button per `ActionCategory`, each expanding a floating
    // sub-row of its `GeneralAction`s (see `run_category_buttons` /
    // `sync_category_menu`).
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                bottom: Val::Px(PANEL_MARGIN),
                justify_content: JustifyContent::Center,
                column_gap: Val::Px(4.0),
                ..default()
            },
            // Hover target for the `click_select` click-through guard.
            Interaction::default(),
        ))
        .with_children(|bar| {
            for category in ActionCategory::ALL {
                bar.spawn(Node {
                    flex_direction: FlexDirection::Column,
                    position_type: PositionType::Relative,
                    align_items: AlignItems::Center,
                    ..default()
                })
                .with_children(|column| {
                    // Floating sub-row: positioned above its own category
                    // button, doesn't affect the column's own height, so all
                    // category buttons stay aligned on one baseline.
                    column
                        .spawn((
                            CategorySubRow { category },
                            Node {
                                position_type: PositionType::Absolute,
                                bottom: Val::Percent(100.0),
                                margin: UiRect::bottom(Val::Px(4.0)),
                                display: Display::None,
                                flex_direction: FlexDirection::Row,
                                column_gap: Val::Px(4.0),
                                ..default()
                            },
                        ))
                        .with_children(|row| {
                            for action in category.actions().iter().copied() {
                                row.spawn((
                                    Button,
                                    GeneralActionButton { action },
                                    Node {
                                        // Squares when the label fits, wider
                                        // when it doesn't ("Clear roof" must
                                        // not spill out of its button).
                                        min_width: Val::Px(ACTION_BUTTON_SIZE),
                                        height: Val::Px(ACTION_BUTTON_SIZE),
                                        padding: UiRect::axes(Val::Px(8.0), Val::Px(0.0)),
                                        justify_content: JustifyContent::Center,
                                        align_items: AlignItems::Center,
                                        ..default()
                                    },
                                    BackgroundColor(BUTTON_BACKGROUND),
                                ))
                                .with_child((
                                    Text::new(action.label()),
                                    TextFont {
                                        font_size: 11.0,
                                        ..default()
                                    },
                                    TextColor(PANEL_TEXT),
                                ));
                            }
                        });

                    column
                        .spawn((
                            Button,
                            CategoryButton { category },
                            Node {
                                min_width: Val::Px(ACTION_BUTTON_SIZE),
                                height: Val::Px(ACTION_BUTTON_SIZE),
                                padding: UiRect::axes(Val::Px(8.0), Val::Px(0.0)),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                ..default()
                            },
                            BackgroundColor(BUTTON_BACKGROUND),
                        ))
                        .with_child((
                            Text::new(category.label()),
                            TextFont {
                                font_size: 11.0,
                                ..default()
                            },
                            TextColor(PANEL_TEXT),
                        ));
                });
            }
        });

    // Top-right allowance panel; rows are added as pawns spawn.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(PANEL_MARGIN),
                top: Val::Px(PANEL_MARGIN),
                padding: UiRect::axes(Val::Px(12.0), Val::Px(8.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(4.0),
                ..default()
            },
            BackgroundColor(PANEL_BACKGROUND),
            // Hover target for the `click_select` click-through guard.
            Interaction::default(),
            AllowancePanel,
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("Allowance"),
                TextFont {
                    font_size: 14.0,
                    ..default()
                },
                TextColor(PANEL_TEXT),
            ));
            // Header row: name-column spacer, then one label per work type.
            parent.spawn(Node::default()).with_children(|header| {
                header.spawn(Node {
                    width: Val::Px(NAME_COL_WIDTH),
                    ..default()
                });
                for (_, label) in ToggleWork::ALL {
                    header
                        .spawn(Node {
                            width: Val::Px(WORK_COL_WIDTH),
                            justify_content: JustifyContent::Center,
                            ..default()
                        })
                        .with_child((
                            Text::new(label),
                            TextFont {
                                font_size: 12.0,
                                ..default()
                            },
                            TextColor(PANEL_TEXT),
                        ));
                }
            });
        });

    // Left-center stock panel: stockpiled totals per category, expandable
    // (click) into per-kind detail rows. The outer strip only centers the
    // panel vertically: no background, and deliberately no `Interaction` —
    // a transparent full-height node must not trip the click-through guard.
    commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            left: Val::Px(PANEL_MARGIN),
            top: Val::Px(0.0),
            bottom: Val::Px(0.0),
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::Center,
            ..default()
        })
        .with_children(|wrapper| {
            wrapper
                .spawn((
                    Node {
                        padding: UiRect::axes(Val::Px(12.0), Val::Px(8.0)),
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(4.0),
                        ..default()
                    },
                    BackgroundColor(PANEL_BACKGROUND),
                    // Hover target for the `click_select` click-through guard.
                    Interaction::default(),
                    // Always sits behind a pawn tab window in the same slot.
                    GlobalZIndex(Z_STOCK),
                ))
                .with_children(|panel| {
                    panel.spawn((
                        Text::new("Stock"),
                        TextFont {
                            font_size: 14.0,
                            ..default()
                        },
                        TextColor(PANEL_TEXT),
                    ));
                    for category in StockCategory::ALL {
                        panel
                            .spawn((
                                Button,
                                StockCategoryRow { category },
                                Node {
                                    width: Val::Px(STOCK_ROW_WIDTH),
                                    justify_content: JustifyContent::SpaceBetween,
                                    ..default()
                                },
                                BackgroundColor(Color::NONE),
                            ))
                            .with_children(|row| {
                                row.spawn((
                                    Text::new(category.label()),
                                    TextFont {
                                        font_size: 12.0,
                                        ..default()
                                    },
                                    TextColor(PANEL_TEXT),
                                ));
                                row.spawn((
                                    Text::new("0"),
                                    TextFont {
                                        font_size: 12.0,
                                        ..default()
                                    },
                                    TextColor(PANEL_TEXT),
                                    StockCategoryAmount { category },
                                ));
                            });
                        // Detail rows are spawned up-front (kinds are known
                        // at startup) and toggled via `Display`, never
                        // respawned.
                        for kind in ItemKind::ALL {
                            if kind.category() != category {
                                continue;
                            }
                            panel
                                .spawn((
                                    StockDetailRow { kind },
                                    Node {
                                        display: Display::None,
                                        margin: UiRect::left(Val::Px(STOCK_DETAIL_INDENT)),
                                        justify_content: JustifyContent::SpaceBetween,
                                        ..default()
                                    },
                                ))
                                .with_children(|row| {
                                    row.spawn((
                                        Text::new(kind.label()),
                                        TextFont {
                                            font_size: 11.0,
                                            ..default()
                                        },
                                        TextColor(PANEL_TEXT),
                                    ));
                                    row.spawn((
                                        Text::new("0"),
                                        TextFont {
                                            font_size: 11.0,
                                            ..default()
                                        },
                                        TextColor(PANEL_TEXT),
                                        StockDetailAmount { kind },
                                    ));
                                });
                        }
                    }
                });
        });

    // Center-left pawn tab window: whichever of Job History / Skills / Needs
    // is active in the info panel's tab bar (see `sync_pawn_tabs`). Same
    // slot as the Stock panel above; the outer wrapper only centers
    // vertically (no background, deliberately no `Interaction`, same reason
    // as the Stock panel wrapper), and the window itself starts hidden —
    // shown only while a pawn is selected and a tab is open.
    commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            left: Val::Px(PANEL_MARGIN),
            top: Val::Px(0.0),
            bottom: Val::Px(0.0),
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::Center,
            ..default()
        })
        .with_children(|wrapper| {
            wrapper
                .spawn((
                    Node {
                        display: Display::None,
                        width: Val::Px(HISTORY_PANEL_WIDTH),
                        padding: UiRect::axes(Val::Px(12.0), Val::Px(8.0)),
                        flex_direction: FlexDirection::Column,
                        ..default()
                    },
                    BackgroundColor(PANEL_BACKGROUND_OPAQUE),
                    // Hover target for the `click_select` click-through guard.
                    Interaction::default(),
                    // Always on top of the Stock panel in the same slot.
                    GlobalZIndex(Z_PAWN_TABS),
                    PawnTabWindow,
                ))
                .with_children(|window| {
                    // Job History: scrollable list, newest first. Fixed
                    // height so it doesn't reflow as entries accumulate;
                    // `Overflow::scroll_y` + `ScrollPosition` clip and scroll
                    // the single `HistoryList` text child (`scroll_history`
                    // drives the mouse wheel). Its own `Interaction` is the
                    // hover check `scroll_history`/`zoom_camera` use.
                    window
                        .spawn((
                            Node {
                                display: Display::None,
                                height: Val::Px(HISTORY_PANEL_HEIGHT),
                                overflow: Overflow::scroll_y(),
                                ..default()
                            },
                            Interaction::default(),
                            ScrollPosition::default(),
                            PawnTabContent(PawnTab::JobHistory),
                        ))
                        .with_child((
                            Text::new(""),
                            TextFont {
                                font_size: 12.0,
                                ..default()
                            },
                            TextColor(PANEL_TEXT),
                            HistoryList,
                        ));
                    // Skills: one row per `Skills` field, tier-letter readout.
                    window
                        .spawn((
                            Node {
                                display: Display::None,
                                flex_direction: FlexDirection::Column,
                                row_gap: Val::Px(4.0),
                                ..default()
                            },
                            PawnTabContent(PawnTab::Skills),
                        ))
                        .with_children(|column| {
                            for skill in SkillKind::ALL {
                                column
                                    .spawn(Node {
                                        width: Val::Percent(100.0),
                                        justify_content: JustifyContent::SpaceBetween,
                                        ..default()
                                    })
                                    .with_children(|row| {
                                        row.spawn((
                                            Text::new(skill.label()),
                                            TextFont {
                                                font_size: 13.0,
                                                ..default()
                                            },
                                            TextColor(PANEL_TEXT),
                                        ));
                                        row.spawn((
                                            Text::new(""),
                                            TextFont {
                                                font_size: 13.0,
                                                ..default()
                                            },
                                            TextColor(PANEL_TEXT),
                                            SkillValueText { skill },
                                        ));
                                    });
                            }
                        });
                    // Needs: one gauge row per `Needs::gauges()` entry (just
                    // Sleep today; a future need shows up here for free).
                    window
                        .spawn((
                            Node {
                                display: Display::None,
                                flex_direction: FlexDirection::Column,
                                row_gap: Val::Px(6.0),
                                ..default()
                            },
                            PawnTabContent(PawnTab::Needs),
                        ))
                        .with_children(|column| {
                            for (index, (label, _)) in Needs::default().gauges().enumerate() {
                                column
                                    .spawn(Node {
                                        align_items: AlignItems::Center,
                                        column_gap: Val::Px(6.0),
                                        ..default()
                                    })
                                    .with_children(|row| {
                                        row.spawn((
                                            Text::new(label),
                                            TextFont {
                                                font_size: 13.0,
                                                ..default()
                                            },
                                            TextColor(PANEL_TEXT),
                                        ));
                                        row.spawn((
                                            Node {
                                                width: Val::Px(GAUGE_WIDTH),
                                                height: Val::Px(GAUGE_HEIGHT),
                                                ..default()
                                            },
                                            BackgroundColor(GAUGE_TRACK_COLOR),
                                        ))
                                        .with_children(
                                            |track| {
                                                track.spawn((
                                                    Node {
                                                        width: Val::Percent(100.0),
                                                        height: Val::Percent(100.0),
                                                        ..default()
                                                    },
                                                    BackgroundColor(GAUGE_FILL_HIGH_COLOR),
                                                    NeedsTabGaugeFill { index },
                                                ));
                                            },
                                        );
                                        row.spawn((
                                            Text::new(""),
                                            TextFont {
                                                font_size: 13.0,
                                                ..default()
                                            },
                                            TextColor(PANEL_TEXT),
                                            NeedsTabGaugeValue { index },
                                        ));
                                    });
                            }
                        });
                });
        });

    // Cursor tooltip: hovered-tile info.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
                ..default()
            },
            BackgroundColor(PANEL_BACKGROUND),
            Visibility::Hidden,
            TooltipPanel,
        ))
        .with_child((
            Text::new(""),
            TextFont {
                font_size: 12.0,
                ..default()
            },
            TextColor(PANEL_TEXT),
            TooltipText,
        ));

    // Right-side pawn job debug panel.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(PANEL_MARGIN),
                top: Val::Percent(40.0),
                padding: UiRect::axes(Val::Px(12.0), Val::Px(8.0)),
                ..default()
            },
            BackgroundColor(PANEL_BACKGROUND),
        ))
        .with_child((
            Text::new(""),
            TextFont {
                font_size: 13.0,
                ..default()
            },
            TextColor(PANEL_TEXT),
            JobPanelText,
        ));

    // Top-center pawn selector bar (chips added per pawn as they spawn).
    // `Interaction` is the usual click-through guard.
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(PANEL_MARGIN),
            left: Val::Px(0.0),
            right: Val::Px(0.0),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::FlexStart,
            column_gap: Val::Px(10.0),
            ..default()
        },
        Interaction::default(),
        PortraitBar,
    ));

    // Top-center "Paused" banner, below the portrait bar. No `Interaction`:
    // it must never block the click-through guard other panels use.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(PANEL_MARGIN + PORTRAIT_SIZE + 32.0),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
            Visibility::Hidden,
            PauseBanner,
        ))
        .with_children(|parent| {
            parent
                .spawn((
                    Node {
                        padding: UiRect::axes(Val::Px(14.0), Val::Px(6.0)),
                        ..default()
                    },
                    BackgroundColor(PANEL_BACKGROUND),
                ))
                .with_child((
                    Text::new("Paused"),
                    TextFont {
                        font_size: 18.0,
                        ..default()
                    },
                    TextColor(PANEL_TEXT),
                ));
        });

    // Top-left day/time chip: the live sky widget (a second camera renders
    // the sun/moon arc to this texture) above the clock text, above the
    // Pause/speed row. The outer column itself carries no `Interaction`
    // (like the pause banner it would otherwise block the click-through
    // guard); the speed row below is real buttons, which bring their own.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(PANEL_MARGIN),
                left: Val::Px(PANEL_MARGIN),
                padding: UiRect::axes(Val::Px(8.0), Val::Px(8.0)),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(6.0),
                ..default()
            },
            BackgroundColor(PANEL_BACKGROUND),
        ))
        .with_children(|chip| {
            chip.spawn((
                ImageNode::new(sky_image.0.clone()),
                Node {
                    width: Val::Px(SKY_WIDGET_WIDTH),
                    height: Val::Px(SKY_WIDGET_HEIGHT),
                    ..default()
                },
            ));
            chip.spawn((
                Text::new(""),
                TextFont {
                    font_size: 14.0,
                    ..default()
                },
                TextColor(PANEL_TEXT),
                ClockLabel,
            ));
            // Pause / 0.5x / 1x / 2x / 3x speed row. Its own `Interaction`
            // guards the small gaps between buttons so hovering it never
            // leaks a click through to the map underneath.
            chip.spawn((
                Node {
                    column_gap: Val::Px(4.0),
                    ..default()
                },
                Interaction::default(),
            ))
            .with_children(|row| {
                row.spawn((
                    Button,
                    SpeedPauseButton,
                    Node {
                        padding: UiRect::axes(Val::Px(6.0), Val::Px(3.0)),
                        ..default()
                    },
                    BackgroundColor(BUTTON_BACKGROUND),
                ))
                .with_child((
                    Text::new("Pause"),
                    TextFont {
                        font_size: 11.0,
                        ..default()
                    },
                    TextColor(PANEL_TEXT),
                ));
                for mult in [0.5, 1.0, 2.0, 3.0] {
                    row.spawn((
                        Button,
                        SpeedButton(mult),
                        Node {
                            padding: UiRect::axes(Val::Px(6.0), Val::Px(3.0)),
                            ..default()
                        },
                        BackgroundColor(BUTTON_BACKGROUND),
                    ))
                    .with_child((
                        Text::new(format!("{mult}x")),
                        TextFont {
                            font_size: 11.0,
                            ..default()
                        },
                        TextColor(PANEL_TEXT),
                    ));
                }
            });
        });
}

/// Keep the corner clock in sync with the `GameClock`.
pub fn update_clock_label(clock: Res<GameClock>, label: Single<&mut Text, With<ClockLabel>>) {
    let text = clock.clock_label();
    let mut label = label.into_inner();
    if label.0 != text {
        label.0 = text;
    }
}

/// Hover/press feedback for the stock category rows; a press toggles the
/// category's per-kind detail rows.
pub fn run_stock_rows(
    mut state: ResMut<StockUiState>,
    mut interactions: Query<
        (&StockCategoryRow, &Interaction, &mut BackgroundColor),
        Changed<Interaction>,
    >,
) {
    for (row, interaction, mut background) in &mut interactions {
        background.0 = match interaction {
            Interaction::Pressed => STOCK_ROW_PRESSED,
            Interaction::Hovered => STOCK_ROW_HOVER,
            Interaction::None => Color::NONE,
        };
        if *interaction != Interaction::Pressed {
            continue;
        }
        if !state.expanded.remove(&row.category) {
            state.expanded.insert(row.category);
        }
    }
}

/// Keep the stock panel at the stockpiled totals (loose piles and carried
/// loads only count once deposited) and its detail rows shown per the
/// expansion state.
pub fn sync_stock_panel(
    stockpile: Res<map::Stockpile>,
    state: Res<StockUiState>,
    stacks: Query<(&ItemStack, &GridCoords)>,
    mut category_amounts: Query<(&StockCategoryAmount, &mut Text)>,
    mut detail_amounts: Query<(&StockDetailAmount, &mut Text), Without<StockCategoryAmount>>,
    mut detail_rows: Query<(&StockDetailRow, &mut Node)>,
) {
    let totals = items::stock_totals(
        stacks
            .iter()
            .map(|(stack, grid)| (stack.kind, stack.amount, stockpile.cells.contains(grid))),
    );
    for (amount, mut text) in &mut category_amounts {
        let total: u32 = ItemKind::ALL
            .iter()
            .filter(|kind| kind.category() == amount.category)
            .map(|kind| totals.get(kind).copied().unwrap_or(0))
            .sum();
        let wanted = total.to_string();
        if text.0 != wanted {
            text.0 = wanted;
        }
    }
    for (amount, mut text) in &mut detail_amounts {
        let wanted = totals.get(&amount.kind).copied().unwrap_or(0).to_string();
        if text.0 != wanted {
            text.0 = wanted;
        }
    }
    for (row, mut node) in &mut detail_rows {
        let display = if state.expanded.contains(&row.kind.category()) {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
    }
}

/// Show the "Paused" banner while the simulation is frozen.
pub fn sync_pause_banner(
    state: Res<State<SimState>>,
    banner: Single<&mut Visibility, With<PauseBanner>>,
) {
    *banner.into_inner() = if *state.get() == SimState::Paused {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };
}

/// A speed button sets `GameSpeed` and resumes play, Sims/Anno-style:
/// picking a speed is itself an "unpause".
pub fn run_speed_buttons(
    interactions: Query<(&Interaction, &SpeedButton), Changed<Interaction>>,
    mut speed: ResMut<GameSpeed>,
    mut next_state: ResMut<NextState<SimState>>,
) {
    for (interaction, button) in &interactions {
        if *interaction == Interaction::Pressed {
            speed.0 = button.0;
            next_state.set(SimState::Running);
        }
    }
}

/// The row's Pause button flips `SimState`, mirroring `game::toggle_pause`
/// so it and the Space bar always agree.
pub fn run_speed_pause_button(
    interactions: Query<&Interaction, (Changed<Interaction>, With<SpeedPauseButton>)>,
    state: Res<State<SimState>>,
    mut next_state: ResMut<NextState<SimState>>,
) {
    for interaction in &interactions {
        if *interaction == Interaction::Pressed {
            next_state.set(match state.get() {
                SimState::Running => SimState::Paused,
                SimState::Paused => SimState::Running,
            });
        }
    }
}

/// Tint the active speed (or Pause, while paused) `BUTTON_PRESSED`; every
/// other control in the row stays `BUTTON_BACKGROUND`.
pub fn sync_speed_buttons(
    state: Res<State<SimState>>,
    speed: Res<GameSpeed>,
    mut speed_buttons: Query<(&SpeedButton, &mut BackgroundColor)>,
    pause_button: Single<&mut BackgroundColor, (With<SpeedPauseButton>, Without<SpeedButton>)>,
) {
    let paused = *state.get() == SimState::Paused;
    for (button, mut background) in &mut speed_buttons {
        let wanted = if !paused && button.0 == speed.0 {
            BUTTON_PRESSED
        } else {
            BUTTON_BACKGROUND
        };
        if background.0 != wanted {
            background.0 = wanted;
        }
    }
    let wanted = if paused {
        BUTTON_PRESSED
    } else {
        BUTTON_BACKGROUND
    };
    let mut pause_background = pause_button.into_inner();
    if pause_background.0 != wanted {
        pause_background.0 = wanted;
    }
}

/// One selector chip per pawn as it spawns: a pawn-colored square with the
/// name's initial, the name underneath.
pub fn spawn_portraits(
    mut commands: Commands,
    bar: Single<Entity, With<PortraitBar>>,
    new_pawns: Query<(Entity, &Name), Added<Pawn>>,
) {
    for (pawn, name) in &new_pawns {
        let initial: String = name.as_str().chars().take(1).collect();
        commands.entity(*bar).with_children(|bar| {
            bar.spawn(Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(2.0),
                ..default()
            })
            .with_children(|chip| {
                chip.spawn((
                    Button,
                    PortraitButton(pawn),
                    Node {
                        width: Val::Px(PORTRAIT_SIZE),
                        height: Val::Px(PORTRAIT_SIZE),
                        border: UiRect::all(Val::Px(PORTRAIT_BORDER)),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    BackgroundColor(PAWN_COLOR),
                    BorderColor::all(PORTRAIT_IDLE_BORDER),
                ))
                .with_child((
                    Text::new(initial),
                    TextFont {
                        font_size: 18.0,
                        ..default()
                    },
                    TextColor(PORTRAIT_TEXT),
                ));
                chip.spawn((
                    Text::new(name.to_string()),
                    TextFont {
                        font_size: 11.0,
                        ..default()
                    },
                    TextColor(PANEL_TEXT),
                ));
            });
        });
    }
}

/// Portrait clicks: select the pawn; a click on the already-selected pawn
/// centers the camera on it instead.
pub fn run_portraits(
    interactions: Query<(&Interaction, &PortraitButton), Changed<Interaction>>,
    mut selected: ResMut<SelectedEntity>,
    pawns: Query<&Transform, (With<Pawn>, Without<MainCamera>)>,
    camera: Single<&mut Transform, With<MainCamera>>,
) {
    let mut camera = camera.into_inner();
    for (interaction, button) in &interactions {
        if *interaction != Interaction::Pressed {
            continue;
        }
        if selected.0 == Some(button.0) {
            if let Ok(pawn_transform) = pawns.get(button.0) {
                scene::center_camera_on(&mut camera, pawn_transform.translation);
            }
        } else {
            selected.0 = Some(button.0);
        }
    }
}

/// Highlight the selected pawn's chip.
pub fn sync_portraits(
    selected: Res<SelectedEntity>,
    mut chips: Query<(&PortraitButton, &mut BorderColor)>,
) {
    for (button, mut border) in &mut chips {
        let color = if selected.0 == Some(button.0) {
            PORTRAIT_SELECTED_BORDER
        } else {
            PORTRAIT_IDLE_BORDER
        };
        *border = BorderColor::all(color);
    }
}

/// Rebuild the job panel text: one "Name: activity" line per pawn.
pub fn update_job_panel(
    pawns: Query<(&Name, &PawnStatus), With<Pawn>>,
    panel_text: Single<&mut Text, With<JobPanelText>>,
) {
    let mut lines: Vec<String> = pawns
        .iter()
        .map(|(name, status)| format!("{}: {}", name, status.0))
        .collect();
    lines.sort();
    panel_text.into_inner().0 = lines.join("\n");
}

/// Follow the cursor with the hovered tile's terrain type, plus the armed
/// zone tool's name if one is active; hidden while hovering UI or off the
/// map.
#[allow(clippy::type_complexity)]
#[allow(clippy::too_many_arguments)]
pub fn update_tooltip(
    window: Single<&Window>,
    camera: Single<(&Camera, &GlobalTransform), With<MainCamera>>,
    terrain: Res<TerrainMap>,
    construction: Res<ConstructionMap>,
    stockpile: Res<map::Stockpile>,
    cover: Res<CoverMap>,
    water: Res<map::WaterMap>,
    // Air reads bundled into one tuple param (system-param arity).
    air: (
        Res<HumidityMap>,
        Res<WindExposureMap>,
        Res<temperature::TemperatureMap>,
        Query<(&ZoneRegion, &temperature::Room)>,
        Res<crate::snow::SnowMap>,
        Res<crate::snow::FreezeLevel>,
    ),
    roofs: Res<RoofMap>,
    tool: Res<ActiveTool>,
    // Plant queries bundled into one tuple param (system-param arity).
    plant_queries: (
        Query<(&Tree, &GridCoords)>,
        Query<(&BerryBush, &GridCoords)>,
        Query<(&Crop, &GridCoords)>,
        Query<(&Shrub, &GridCoords)>,
    ),
    fire_state: (Res<crate::fire::FireMap>, Res<crate::fire::ScorchMap>),
    ui_nodes: Query<&Interaction, Without<TooltipPanel>>,
    tooltip: Single<(&mut Node, &mut Visibility), With<TooltipPanel>>,
    text: Single<&mut Text, With<TooltipText>>,
) {
    let (mut node, mut visibility) = tooltip.into_inner();
    let (camera, camera_transform) = *camera;

    let over_ui = ui_nodes
        .iter()
        .any(|interaction| *interaction != Interaction::None);
    let hovered = (!over_ui)
        .then(|| {
            let cursor = window.cursor_position()?;
            let cell = cursor_to_cell(&window, camera, camera_transform)?;
            let kind = terrain.get(cell)?;
            // A built cell takes over the primary label ("Wall" rather
            // than the "Dirt" it's actually standing on) — ground terrain
            // no longer changes when something is built on it, so without
            // this the tooltip would just show the natural ground. A
            // stockpile cell is the same story: its terrain is plain Dirt,
            // with the zone tracked separately in `map::Stockpile`. And so
            // is water: a river cell's terrain is Dirt (the bed), the
            // player-facing "Shallow/Deep water" comes from the live
            // column (`map::surface_label`).
            let (humidity, exposure, temps, rooms, snow, freeze) = &air;
            let name = construction
                .get(cell)
                .map(|k| k.label())
                .unwrap_or_else(|| {
                    if stockpile.cells.contains(&cell) {
                        "Stockpile"
                    } else {
                        map::surface_label(kind, water.depth(cell), freeze.is_frozen())
                    }
                });
            // "Dirt, grass 80%" when cover sits on the cell (still true on
            // a built cell that had grass before it was built over).
            let mut label = match cover.get(cell) {
                Some((flora, coverage)) => format!(
                    "{}, {} {}%",
                    name,
                    flora.label(),
                    (coverage * 100.0).round()
                ),
                None => name.to_string(),
            };
            let wetness = humidity.get(cell).unwrap_or(0.0);
            if wetness >= 0.01 {
                label = format!("{label}, humidity {}%", (wetness * 100.0).round());
            }
            let cover_depth = snow.get(cell).unwrap_or(0.0);
            if cover_depth >= 0.01 {
                label = format!("{label}, snow {}%", (cover_depth * 100.0).round());
            }
            // Unconditional: wind is permanent, and "wind 100%" vs "15%"
            // behind a wall is the readable gradient. Ground band: this is
            // what a pawn standing here feels, not a turbine's rotor.
            if let Some(share) = exposure.get(cell, WindBand::Ground) {
                label = format!("{label}, wind {}%", (share * 100.0).round());
            }
            // Unconditional too: every cell has a temperature, and watching
            // it climb toward noon (or lag behind, indoors) is the point.
            if let Some(celsius) = temps.get(cell) {
                label = format!("{label}, {celsius:.1}\u{b0}C");
            }
            if let Some((_, room)) = rooms.iter().find(|(region, _)| region.contains(cell)) {
                label = format!("{label}, insulated {}%", (room.insulation * 100.0).round());
            }
            let (trees, bushes, crops, shrubs) = &plant_queries;
            let plant_fuel = cover::plant_fuel_at(
                cell,
                trees
                    .iter()
                    .map(|(tree, grid)| (*grid, tree.fuel()))
                    .chain(bushes.iter().map(|(bush, grid)| (*grid, bush.fuel())))
                    .chain(crops.iter().map(|(crop, grid)| (*grid, crop.fuel())))
                    .chain(shrubs.iter().map(|(shrub, grid)| (*grid, shrub.fuel()))),
            );
            // Player-facing wording: "flammable", not the sim's "fuel"
            // jargon (a proper tooltip UX pass is planned).
            let fuel = cover::fuel_at(cell, &terrain, &construction, &cover, &water, plant_fuel);
            if fuel > 0.0 {
                label = format!("{label}, flammable {}%", (fuel * 100.0).round());
            }
            // Player-facing fire state: no intensities, no numbers.
            let (fire_map, scorch_map) = &fire_state;
            if fire_map.is_burning(cell) {
                label = format!("{label}, on fire");
            } else if scorch_map.get(cell) >= 0.1 {
                label = format!("{label}, scorched");
            }
            if roofs.is_roofed(cell) {
                label = format!("{label}, roofed");
            } else if roofs.policy(cell) == RoofPolicy::Roof {
                label = format!("{label}, roof planned");
            }
            if roofs.policy(cell) == RoofPolicy::NoRoof {
                label = format!("{label}, no-roof area");
            }
            Some((cursor, label))
        })
        .flatten();

    let Some((cursor, label)) = hovered else {
        *visibility = Visibility::Hidden;
        return;
    };
    node.left = Val::Px(cursor.x + TOOLTIP_OFFSET.x);
    node.top = Val::Px(cursor.y + TOOLTIP_OFFSET.y);
    text.into_inner().0 = match tool.0 {
        Some(active) => format!("{label}\n{}", active.label()),
        None => label,
    };
    *visibility = Visibility::Visible;
}

/// One allowance row per pawn: name plus a checkbox per work type.
pub fn spawn_allowance_rows(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    panel: Single<Entity, With<AllowancePanel>>,
    new_pawns: Query<(Entity, &Name), Added<Pawn>>,
) {
    for (pawn, name) in &new_pawns {
        commands.entity(*panel).with_children(|parent| {
            parent
                .spawn((
                    Node {
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    AllowanceRow(pawn),
                ))
                .with_children(|row| {
                    row.spawn((
                        Node {
                            width: Val::Px(NAME_COL_WIDTH),
                            ..default()
                        },
                        Text::new(name.to_string()),
                        TextFont {
                            font_size: 13.0,
                            ..default()
                        },
                        TextColor(PANEL_TEXT),
                    ));
                    // One centered checkbox per work-type column.
                    for (work, _) in ToggleWork::ALL {
                        row.spawn(Node {
                            width: Val::Px(WORK_COL_WIDTH),
                            justify_content: JustifyContent::Center,
                            ..default()
                        })
                        .with_children(|cell| {
                            cell.spawn((
                                Button,
                                AllowanceToggle { pawn, work },
                                Node {
                                    width: Val::Px(CHECKBOX_SIZE),
                                    height: Val::Px(CHECKBOX_SIZE),
                                    border: UiRect::all(Val::Px(1.0)),
                                    justify_content: JustifyContent::Center,
                                    align_items: AlignItems::Center,
                                    ..default()
                                },
                                BorderColor::all(CHECKBOX_BORDER),
                                BackgroundColor(CHECKBOX_ON_COLOR),
                            ))
                            .with_child((
                                Text::new(CHECKBOX_MARK),
                                TextFont {
                                    font: asset_server.load(CHECKBOX_MARK_FONT),
                                    font_size: 14.0,
                                    ..default()
                                },
                                TextColor(PANEL_TEXT),
                                CheckGlyph,
                            ));
                        });
                    }
                });
        });
    }
}

/// A press on a toggle flips that work type on the pawn's `Allowance`.
pub fn toggle_allowance(
    toggles: Query<(&Interaction, &AllowanceToggle), Changed<Interaction>>,
    mut allowances: Query<&mut Allowance, With<Pawn>>,
) {
    for (interaction, toggle) in &toggles {
        if *interaction != Interaction::Pressed {
            continue;
        }
        if let Ok(mut allowance) = allowances.get_mut(toggle.pawn) {
            match toggle.work {
                ToggleWork::Harvest => allowance.harvest = !allowance.harvest,
                ToggleWork::Haul => allowance.haul = !allowance.haul,
                ToggleWork::Farming => allowance.farm = !allowance.farm,
                ToggleWork::Forestry => allowance.forestry = !allowance.forestry,
                ToggleWork::Build => allowance.build = !allowance.build,
            }
        }
    }
}

/// Show/hide each checkbox's mark glyph from its pawn's current allowance
/// (the fill color is only a secondary cue), with a lighter tint on hover;
/// drop rows whose pawn is gone.
pub fn sync_allowance_buttons(
    mut commands: Commands,
    mut toggles: Query<(
        &AllowanceToggle,
        &Interaction,
        &mut BackgroundColor,
        &Children,
    )>,
    mut glyphs: Query<&mut Visibility, With<CheckGlyph>>,
    rows: Query<(Entity, &AllowanceRow)>,
    allowances: Query<&Allowance, With<Pawn>>,
) {
    for (toggle, interaction, mut background, children) in &mut toggles {
        let Ok(allowance) = allowances.get(toggle.pawn) else {
            continue;
        };
        let on = match toggle.work {
            ToggleWork::Harvest => allowance.harvest,
            ToggleWork::Haul => allowance.haul,
            ToggleWork::Farming => allowance.farm,
            ToggleWork::Forestry => allowance.forestry,
            ToggleWork::Build => allowance.build,
        };
        let hovered = *interaction != Interaction::None;
        background.0 = match (on, hovered) {
            (true, false) => CHECKBOX_ON_COLOR,
            (true, true) => CHECKBOX_ON_HOVER,
            (false, false) => CHECKBOX_OFF_COLOR,
            (false, true) => CHECKBOX_OFF_HOVER,
        };
        for child in children {
            if let Ok(mut visibility) = glyphs.get_mut(*child) {
                *visibility = if on {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                };
            }
        }
    }
    for (row, AllowanceRow(pawn)) in &rows {
        if allowances.get(*pawn).is_err() {
            commands.entity(row).despawn();
        }
    }
}

/// Show the Grow row only while a growing zone is selected, and reflect its
/// `GrowOrder.enabled` on the checkbox (fill color + mark glyph, same cue
/// scheme as the Allowance checkboxes).
#[allow(clippy::type_complexity)]
pub fn sync_grow_checkbox(
    selected: Res<SelectedEntity>,
    growing_zones: Query<&GrowOrder>,
    mut rows: Query<&mut Node, With<GrowRow>>,
    mut checkbox: Query<(&Interaction, &mut BackgroundColor, &Children), With<GrowCheckbox>>,
    mut glyphs: Query<&mut Visibility, With<CheckGlyph>>,
) {
    let grow = selected.0.and_then(|entity| growing_zones.get(entity).ok());
    for mut node in &mut rows {
        node.display = if grow.is_some() {
            Display::Flex
        } else {
            Display::None
        };
    }
    let Some(grow) = grow else {
        return;
    };
    for (interaction, mut background, children) in &mut checkbox {
        let hovered = *interaction != Interaction::None;
        background.0 = match (grow.enabled, hovered) {
            (true, false) => CHECKBOX_ON_COLOR,
            (true, true) => CHECKBOX_ON_HOVER,
            (false, false) => CHECKBOX_OFF_COLOR,
            (false, true) => CHECKBOX_OFF_HOVER,
        };
        for child in children {
            if let Ok(mut visibility) = glyphs.get_mut(*child) {
                *visibility = if grow.enabled {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                };
            }
        }
    }
}

/// A press on the Grow checkbox flips the selected zone's `GrowOrder`.
pub fn toggle_grow_order(
    selected: Res<SelectedEntity>,
    checkbox: Query<&Interaction, (With<GrowCheckbox>, Changed<Interaction>)>,
    mut growing_zones: Query<&mut GrowOrder>,
) {
    let Ok(interaction) = checkbox.single() else {
        return;
    };
    if *interaction != Interaction::Pressed {
        return;
    }
    if let Some(mut grow) = selected
        .0
        .and_then(|entity| growing_zones.get_mut(entity).ok())
    {
        grow.enabled = !grow.enabled;
    }
}

/// Show the Manual row only while a pawn is selected, and reflect whether it
/// currently carries `ManualMode` on the checkbox (fill color + mark glyph,
/// same cue scheme as the Grow/Allowance checkboxes).
#[allow(clippy::type_complexity)]
/// Linearly interpolate the gauge fill color from low (`t = 0`) to high
/// (`t = 1`), `t` already clamped by the caller.
fn gauge_fill_color(t: f32) -> Color {
    let low = GAUGE_FILL_LOW_COLOR.to_srgba();
    let high = GAUGE_FILL_HIGH_COLOR.to_srgba();
    Color::srgb(
        low.red + (high.red - low.red) * t,
        low.green + (high.green - low.green) * t,
        low.blue + (high.blue - low.blue) * t,
    )
}

/// Show the Sleep gauge only while a pawn is selected; keep its fill width,
/// color, and numeric readout tracking `Needs::sleep`. Writes are guarded by
/// inequality checks so the fill/text only change when the value actually
/// moves (per the UI-systems-run-every-frame pitfall).
#[allow(clippy::type_complexity)]
pub fn sync_needs_panel(
    selected: Res<SelectedEntity>,
    pawns: Query<&Needs, With<Pawn>>,
    mut rows: Query<&mut Node, With<NeedsSection>>,
    mut fill: Query<
        (&mut Node, &mut BackgroundColor),
        (With<SleepGaugeFill>, Without<NeedsSection>),
    >,
    mut value: Query<&mut Text, With<SleepGaugeValue>>,
) {
    let needs = selected.0.and_then(|entity| pawns.get(entity).ok());
    for mut node in &mut rows {
        let wanted = if needs.is_some() {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != wanted {
            node.display = wanted;
        }
    }
    let Some(needs) = needs else {
        return;
    };
    let sleep = needs.sleep.clamp(0.0, 100.0);
    let wanted_width = Val::Percent(sleep);
    let wanted_color = gauge_fill_color(sleep / 100.0);
    for (mut node, mut background) in &mut fill {
        if node.width != wanted_width {
            node.width = wanted_width;
        }
        if background.0 != wanted_color {
            background.0 = wanted_color;
        }
    }
    let wanted_text = (sleep.round() as i32).to_string();
    for mut text in &mut value {
        if text.0 != wanted_text {
            text.0 = wanted_text.clone();
        }
    }
}

pub fn sync_manual_checkbox(
    selected: Res<SelectedEntity>,
    pawns: Query<Has<ManualMode>, With<Pawn>>,
    mut rows: Query<&mut Node, With<ManualRow>>,
    mut checkbox: Query<(&Interaction, &mut BackgroundColor, &Children), With<ManualCheckbox>>,
    mut glyphs: Query<&mut Visibility, With<CheckGlyph>>,
) {
    let manual = selected.0.and_then(|entity| pawns.get(entity).ok());
    for mut node in &mut rows {
        node.display = if manual.is_some() {
            Display::Flex
        } else {
            Display::None
        };
    }
    let Some(manual) = manual else {
        return;
    };
    for (interaction, mut background, children) in &mut checkbox {
        let hovered = *interaction != Interaction::None;
        background.0 = match (manual, hovered) {
            (true, false) => CHECKBOX_ON_COLOR,
            (true, true) => CHECKBOX_ON_HOVER,
            (false, false) => CHECKBOX_OFF_COLOR,
            (false, true) => CHECKBOX_OFF_HOVER,
        };
        for child in children {
            if let Ok(mut visibility) = glyphs.get_mut(*child) {
                *visibility = if manual {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                };
            }
        }
    }
}

/// A press on the Manual checkbox drafts/undrafts the selected pawn — takes
/// it off (or back onto) AI control.
pub fn toggle_manual_mode(
    mut commands: Commands,
    selected: Res<SelectedEntity>,
    checkbox: Query<&Interaction, (With<ManualCheckbox>, Changed<Interaction>)>,
    pawns: Query<Has<ManualMode>, With<Pawn>>,
) {
    let Ok(interaction) = checkbox.single() else {
        return;
    };
    if *interaction != Interaction::Pressed {
        return;
    }
    let Some(pawn) = selected.0 else {
        return;
    };
    let Ok(manual) = pawns.get(pawn) else {
        return;
    };
    if manual {
        commands.entity(pawn).remove::<ManualMode>();
    } else {
        commands.entity(pawn).insert(ManualMode);
    }
}

/// Show the cooler target-temperature row only while a `Cooler` is selected,
/// and keep its readout tracking `Cooler::target_c` (guarded by an
/// inequality check, the same UI-systems-run-every-frame pitfall
/// `sync_needs_panel` follows). Button hover/press feedback follows the same
/// `BUTTON_*` cue `run_general_actions` uses — plain, since unlike a
/// checkbox there's no on/off state to shape-encode.
#[allow(clippy::type_complexity)]
pub fn sync_target_temp_row(
    selected: Res<SelectedEntity>,
    coolers: Query<&Cooler>,
    mut rows: Query<&mut Node, With<TargetTempRow>>,
    mut buttons: Query<(&Interaction, &mut BackgroundColor), With<TargetTempButton>>,
    mut value: Query<&mut Text, With<TargetTempValue>>,
) {
    let cooler = selected.0.and_then(|entity| coolers.get(entity).ok());
    for mut node in &mut rows {
        let wanted = if cooler.is_some() {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != wanted {
            node.display = wanted;
        }
    }
    for (interaction, mut background) in &mut buttons {
        background.0 = match interaction {
            Interaction::Pressed => BUTTON_PRESSED,
            Interaction::Hovered => BUTTON_HOVER,
            Interaction::None => BUTTON_BACKGROUND,
        };
    }
    let Some(cooler) = cooler else {
        return;
    };
    let wanted_text = format!("{:.0}C", cooler.target_c);
    for mut text in &mut value {
        if text.0 != wanted_text {
            text.0 = wanted_text.clone();
        }
    }
}

/// A press on either target-temperature nudger steps the selected cooler's
/// `target_c` by `TARGET_TEMP_STEP`, clamped to `TARGET_TEMP_MIN..=MAX` — the
/// same nudger-handler shape as `debug_ui::run_water_level_buttons`.
pub fn run_target_temp_buttons(
    selected: Res<SelectedEntity>,
    buttons: Query<(&Interaction, &TargetTempButton), Changed<Interaction>>,
    mut coolers: Query<&mut Cooler>,
) {
    let Some(entity) = selected.0 else {
        return;
    };
    for (interaction, button) in &buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }
        // Only borrowed mutably on an actual press, so a quiet frame never
        // trips `Changed<Cooler>` for nothing.
        let Ok(mut cooler) = coolers.get_mut(entity) else {
            return;
        };
        let step = if button.raise {
            TARGET_TEMP_STEP
        } else {
            -TARGET_TEMP_STEP
        };
        cooler.target_c = (cooler.target_c + step).clamp(TARGET_TEMP_MIN, TARGET_TEMP_MAX);
    }
}

/// Tab-bar clicks toggle the active tab: press an inactive tab to open it
/// (and switch to it), press the already-active tab to close the window.
pub fn run_pawn_tab_buttons(
    interactions: Query<(&Interaction, &PawnTabButton), Changed<Interaction>>,
    mut active: ResMut<ActivePawnTab>,
) {
    for (interaction, button) in &interactions {
        if *interaction == Interaction::Pressed {
            active.0 = if active.0 == Some(button.0) {
                None
            } else {
                Some(button.0)
            };
        }
    }
}

/// Show the tab bar only while a pawn is selected; tint the active tab's
/// button, and show/hide `PawnTabWindow` and its `PawnTabContent` columns to
/// match `ActivePawnTab`. Mirrors `debug_ui::sync_tabs`, ANDed with the
/// pawn-selected check `sync_needs_panel` and friends use.
#[allow(clippy::type_complexity)]
pub fn sync_pawn_tabs(
    selected: Res<SelectedEntity>,
    active: Res<ActivePawnTab>,
    pawns: Query<(), With<Pawn>>,
    mut bars: Query<
        &mut Node,
        (
            With<PawnTabBar>,
            Without<PawnTabWindow>,
            Without<PawnTabContent>,
        ),
    >,
    mut windows: Query<
        &mut Node,
        (
            With<PawnTabWindow>,
            Without<PawnTabBar>,
            Without<PawnTabContent>,
        ),
    >,
    mut contents: Query<
        (&PawnTabContent, &mut Node),
        (Without<PawnTabBar>, Without<PawnTabWindow>),
    >,
    mut buttons: Query<(&PawnTabButton, &mut BackgroundColor)>,
) {
    let is_pawn = selected.0.is_some_and(|entity| pawns.get(entity).is_ok());
    for mut node in &mut bars {
        node.display = if is_pawn {
            Display::Flex
        } else {
            Display::None
        };
    }
    let visible = is_pawn && active.0.is_some();
    if let Ok(mut node) = windows.single_mut() {
        node.display = if visible {
            Display::Flex
        } else {
            Display::None
        };
    }
    for (content, mut node) in &mut contents {
        node.display = if Some(content.0) == active.0 {
            Display::Flex
        } else {
            Display::None
        };
    }
    for (button, mut background) in &mut buttons {
        background.0 = if Some(button.0) == active.0 {
            BUTTON_PRESSED
        } else {
            BUTTON_BACKGROUND
        };
    }
}

/// Render the selected pawn's job history (newest entry first, per
/// `JobHistory`'s own ordering) into the Job History tab's `HistoryList` text
/// node. Resets scroll to the top whenever the selection or active tab
/// changes, so switching pawns (or tabs) never leaves a stale scroll offset
/// behind.
pub fn sync_job_history_tab(
    selected: Res<SelectedEntity>,
    active: Res<ActivePawnTab>,
    histories: Query<&JobHistory>,
    mut scroll: Query<&mut ScrollPosition, With<PawnTabContent>>,
    mut list_text: Query<&mut Text, With<HistoryList>>,
    mut last: Local<(Option<Entity>, Option<PawnTab>)>,
) {
    if (selected.0, active.0) != *last {
        if let Ok(mut scroll) = scroll.single_mut() {
            scroll.y = 0.0;
        }
        *last = (selected.0, active.0);
    }
    if active.0 != Some(PawnTab::JobHistory) {
        return;
    }
    let Some(history) = selected.0.and_then(|entity| histories.get(entity).ok()) else {
        return;
    };
    let Ok(mut text) = list_text.single_mut() else {
        return;
    };
    text.0 = if history.0.is_empty() {
        "No completed jobs yet.".to_string()
    } else {
        history
            .0
            .iter()
            .map(|entry| format!("{}  {}", entry.time, entry.label))
            .collect::<Vec<_>>()
            .join("\n")
    };
}

/// Update the Skills tab's tier-letter readouts for the selected pawn. Only
/// bothers while the tab is active; `sync_pawn_tabs` handles hiding it.
pub fn sync_skills_tab(
    selected: Res<SelectedEntity>,
    active: Res<ActivePawnTab>,
    pawns: Query<&Skills, With<Pawn>>,
    mut values: Query<(&SkillValueText, &mut Text)>,
) {
    if active.0 != Some(PawnTab::Skills) {
        return;
    }
    let Some(skills) = selected.0.and_then(|entity| pawns.get(entity).ok()) else {
        return;
    };
    for (value, mut text) in &mut values {
        let wanted = value.skill.tier(skills).letter().to_string();
        if text.0 != wanted {
            text.0 = wanted;
        }
    }
}

/// Update the Needs tab's gauge fills/readouts for the selected pawn, walking
/// `Needs::gauges()` so a future need shows up here without touching this
/// system. Only bothers while the tab is active; `sync_pawn_tabs` handles
/// hiding it.
#[allow(clippy::type_complexity)]
pub fn sync_needs_tab(
    selected: Res<SelectedEntity>,
    active: Res<ActivePawnTab>,
    pawns: Query<&Needs, With<Pawn>>,
    mut fills: Query<(&NeedsTabGaugeFill, &mut Node, &mut BackgroundColor)>,
    mut values: Query<(&NeedsTabGaugeValue, &mut Text)>,
) {
    if active.0 != Some(PawnTab::Needs) {
        return;
    }
    let Some(needs) = selected.0.and_then(|entity| pawns.get(entity).ok()) else {
        return;
    };
    let gauges: Vec<f32> = needs.gauges().map(|(_, value)| value).collect();
    for (fill, mut node, mut background) in &mut fills {
        let Some(&value) = gauges.get(fill.index) else {
            continue;
        };
        let value = value.clamp(0.0, 100.0);
        let wanted_width = Val::Percent(value);
        let wanted_color = gauge_fill_color(value / 100.0);
        if node.width != wanted_width {
            node.width = wanted_width;
        }
        if background.0 != wanted_color {
            background.0 = wanted_color;
        }
    }
    for (value_marker, mut text) in &mut values {
        let Some(&value) = gauges.get(value_marker.index) else {
            continue;
        };
        let wanted = (value.round() as i32).to_string();
        if text.0 != wanted {
            text.0 = wanted;
        }
    }
}

/// Mouse wheel over the Job History tab scrolls its list instead of zooming
/// the camera (`scene::zoom_camera` skips while this is hovered).
pub fn scroll_history(
    scroll_input: Res<AccumulatedMouseScroll>,
    mut content: Query<(&Interaction, &mut ScrollPosition), With<PawnTabContent>>,
) {
    let dy = scroll_input.delta.y;
    if dy == 0.0 {
        return;
    }
    if let Ok((interaction, mut scroll)) = content.single_mut() {
        if *interaction != Interaction::None {
            scroll.y = (scroll.y - dy * HISTORY_SCROLL_SPEED).max(0.0);
        }
    }
}

/// Show the plant-selector row only while a growing zone is selected, with
/// the button's label tracking the zone's current `ZonePlant`.
#[allow(clippy::type_complexity)]
pub fn sync_plant_button(
    selected: Res<SelectedEntity>,
    growing_zones: Query<&ZonePlant>,
    mut rows: Query<&mut Node, With<PlantRow>>,
    mut button: Query<(&Interaction, &mut BackgroundColor), With<PlantButton>>,
    mut label: Query<&mut Text, With<PlantButtonText>>,
) {
    let plant = selected.0.and_then(|entity| growing_zones.get(entity).ok());
    for mut node in &mut rows {
        node.display = if plant.is_some() {
            Display::Flex
        } else {
            Display::None
        };
    }
    let Some(plant) = plant else {
        return;
    };
    for (interaction, mut background) in &mut button {
        background.0 = match interaction {
            Interaction::Pressed => BUTTON_PRESSED,
            Interaction::Hovered => BUTTON_HOVER,
            Interaction::None => BUTTON_BACKGROUND,
        };
    }
    for mut text in &mut label {
        if text.0 != plant.label() {
            text.0 = plant.label().to_string();
        }
    }
}

/// A press on the plant button steps the selected zone's `ZonePlant` to the
/// next choice. Pending Farming jobs for the old plant void via
/// `cleanup_jobs` (their zone no longer wants that kind).
pub fn cycle_zone_plant(
    selected: Res<SelectedEntity>,
    button: Query<&Interaction, (With<PlantButton>, Changed<Interaction>)>,
    mut growing_zones: Query<&mut ZonePlant>,
) {
    let Ok(interaction) = button.single() else {
        return;
    };
    if *interaction != Interaction::Pressed {
        return;
    }
    if let Some(mut plant) = selected
        .0
        .and_then(|entity| growing_zones.get_mut(entity).ok())
    {
        *plant = plant.next();
    }
}

/// One floating name tag per pawn, spawned when the pawn's visual appears.
pub fn spawn_pawn_labels(mut commands: Commands, new_pawns: Query<(Entity, &Name), Added<Pawn>>) {
    for (pawn, name) in &new_pawns {
        commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    width: Val::Px(LABEL_WIDTH),
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                PawnLabel(pawn),
            ))
            .with_child((
                Text::new(name.to_string()),
                TextFont {
                    font_size: 12.0,
                    ..default()
                },
                TextColor(PANEL_TEXT),
            ));
    }
}

/// Keep each name tag just below its pawn's model on screen.
pub fn sync_pawn_labels(
    mut commands: Commands,
    camera: Single<(&Camera, &GlobalTransform), With<MainCamera>>,
    pawns: Query<&GlobalTransform, With<Pawn>>,
    mut labels: Query<(Entity, &PawnLabel, &mut Node, &mut Visibility)>,
) {
    let (camera, camera_transform) = *camera;
    for (label_entity, label, mut node, mut visibility) in &mut labels {
        let Ok(pawn_transform) = pawns.get(label.0) else {
            commands.entity(label_entity).despawn();
            continue;
        };
        let base = pawn_transform.translation() - Vec3::Y * PAWN_HEIGHT / 2.0;
        let Ok(screen) = camera.world_to_viewport(camera_transform, base) else {
            *visibility = Visibility::Hidden;
            continue;
        };
        node.left = Val::Px(screen.x - LABEL_WIDTH / 2.0);
        node.top = Val::Px(screen.y + 4.0);
        *visibility = Visibility::Visible;
    }
}

/// World anchor for a compass letter: a `GridCoords` just past the
/// corresponding map edge, run through the ordinary `map::grid_to_world` so
/// this shares its offset math instead of re-deriving world-space signs.
/// East is `+X` (`daynight::sky_position`'s sunrise direction); north is
/// `grid.y + 1`'s direction, i.e. `-Z` (see the neighbor lookup this mirrors
/// in `on_cell`'s tooltip code).
fn compass_anchor(dir: CompassDir) -> Vec3 {
    let half_w = MAP_WIDTH / 2;
    let half_h = MAP_HEIGHT / 2;
    let grid = match dir {
        CompassDir::North => GridCoords::new(half_w, MAP_HEIGHT + COMPASS_MARGIN_TILES),
        CompassDir::South => GridCoords::new(half_w, -COMPASS_MARGIN_TILES),
        CompassDir::East => GridCoords::new(MAP_WIDTH + COMPASS_MARGIN_TILES, half_h),
        CompassDir::West => GridCoords::new(-COMPASS_MARGIN_TILES, half_h),
    };
    map::grid_to_world(&grid)
}

/// The four N/E/S/W letters just outside the map edges, spawned once at
/// startup — orientation should stay legible even while paused, so this
/// (like `sync_compass_labels`) isn't sim-gated.
pub fn spawn_compass_labels(mut commands: Commands) {
    for (dir, letter) in [
        (CompassDir::North, "N"),
        (CompassDir::East, "E"),
        (CompassDir::South, "S"),
        (CompassDir::West, "W"),
    ] {
        commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    width: Val::Px(LABEL_WIDTH),
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                CompassLabel(dir),
            ))
            .with_child((
                Text::new(letter),
                TextFont {
                    font_size: 16.0,
                    ..default()
                },
                TextColor(PANEL_TEXT),
            ));
    }
}

/// Keep each compass letter projected onto its world anchor — re-derived
/// every frame, so orbiting the camera (`scene::orbit_camera`) keeps the
/// letters honest instead of pinning them to fixed screen corners.
pub fn sync_compass_labels(
    camera: Single<(&Camera, &GlobalTransform), With<MainCamera>>,
    mut labels: Query<(&CompassLabel, &mut Node, &mut Visibility)>,
) {
    let (camera, camera_transform) = *camera;
    for (label, mut node, mut visibility) in &mut labels {
        let anchor = compass_anchor(label.0);
        let Ok(screen) = camera.world_to_viewport(camera_transform, anchor) else {
            *visibility = Visibility::Hidden;
            continue;
        };
        node.left = Val::Px(screen.x - LABEL_WIDTH / 2.0);
        node.top = Val::Px(screen.y - 8.0);
        *visibility = Visibility::Visible;
    }
}

/// Frame the selected entity with a square of `SELECTION_WORLD_SIZE` world
/// units, converted to pixels through the camera projection.
pub fn sync_indicator(
    selected: Res<SelectedEntity>,
    camera: Single<(&Camera, &GlobalTransform), With<MainCamera>>,
    targets: Query<&GlobalTransform, With<Selectable>>,
    indicator: Single<(&mut Node, &mut Visibility), With<SelectionIndicator>>,
) {
    let (mut node, mut visibility) = indicator.into_inner();
    let (camera, camera_transform) = *camera;

    let center = selected
        .0
        .and_then(|entity| targets.get(entity).ok())
        .map(GlobalTransform::translation);
    let Some(center) = center else {
        *visibility = Visibility::Hidden;
        return;
    };

    let (Ok(origin), Ok(offset)) = (
        camera.world_to_viewport(camera_transform, center),
        camera.world_to_viewport(camera_transform, center + *camera_transform.right()),
    ) else {
        *visibility = Visibility::Hidden;
        return;
    };

    let pixels_per_unit = origin.distance(offset);
    let half = SELECTION_WORLD_SIZE * pixels_per_unit / 2.0;
    node.left = Val::Px(origin.x - half);
    node.top = Val::Px(origin.y - half);
    node.width = Val::Px(half * 2.0);
    node.height = Val::Px(half * 2.0);
    *visibility = Visibility::Visible;
}

#[allow(clippy::type_complexity)]
pub fn update_panel(
    selected: Res<SelectedEntity>,
    temps: Res<temperature::TemperatureMap>,
    roofs: Res<RoofMap>,
    rooms: Query<(&ZoneRegion, &temperature::Room)>,
    targets: Query<(
        &Name,
        Option<&BerryBush>,
        Option<&ItemStack>,
        Option<&Crop>,
        Option<(&PawnStatus, &Skills, &Objective)>,
        Option<(&Zone, &ZoneRegion)>,
        Option<&Tree>,
        Option<&Shrub>,
        Option<&Blueprint>,
        Has<ToHarvest>,
        Has<ToCut>,
        Option<&PowerOutput>,
        Option<&Battery>,
        Option<(&GridCoords, &Cooler)>,
    )>,
    hud: Single<&mut Visibility, With<SelectionHud>>,
    name_text: Single<&mut Text, With<InfoName>>,
    detail_text: Single<&mut Text, (With<InfoDetail>, Without<InfoName>)>,
) {
    let mut hud = hud.into_inner();
    let Some((
        name,
        berry_bush,
        berry_stack,
        crop,
        pawn,
        zone,
        tree,
        shrub,
        blueprint,
        to_harvest,
        to_cut,
        power_output,
        battery,
        cooler,
    )) = selected.0.and_then(|entity| targets.get(entity).ok())
    else {
        *hud = Visibility::Hidden;
        return;
    };

    name_text.into_inner().0 = name.to_string();
    detail_text.into_inner().0 = if let Some((cell, cooler)) = cooler {
        match temperature::cooler_room_temperature(*cell, &temps, &roofs, &rooms) {
            Some(room_temp) => {
                let desire =
                    ((room_temp - cooler.target_c).abs() / CLIMATE_FULL_DELTA).clamp(0.0, 1.0);
                format!(
                    "Room: {:.0}C / target {:.0}C - draw {:.1}/s",
                    room_temp,
                    cooler.target_c,
                    CLIMATE_POWER_DRAW * desire
                )
            }
            None => format!("Target {:.0}C - no room to condition", cooler.target_c),
        }
    } else if let Some(blueprint) = blueprint {
        format!(
            "Wood: {} / {}{}",
            blueprint.delivered,
            blueprint.kind.wood_cost(),
            if blueprint.is_supplied() {
                " - ready to build"
            } else {
                ""
            }
        )
    } else if let Some(battery) = battery {
        format!("Charge: {:.0} / {:.0}", battery.charge, BATTERY_CAPACITY)
    } else if let Some(output) = power_output {
        format!("Output: {:.1}/s", output.0)
    } else if let Some(shrub) = shrub {
        // Shrubs only ever grow — no yields, no marks.
        format!("Growth: {}%", (shrub.growth * 100.0).round())
    } else {
        match (berry_bush, berry_stack, crop, pawn, zone, tree) {
            // Plain hyphen: Bevy's built-in default font has no em-dash glyph.
            (Some(bush), ..) if to_harvest => format!(
                "Growth: {}% - marked for harvest (yield {})",
                (bush.growth * 100.0).round(),
                bush.yield_amount()
            ),
            (Some(bush), ..) if bush.is_harvestable() => format!(
                "Growth: {}% - harvestable (yield {})",
                (bush.growth * 100.0).round(),
                bush.yield_amount()
            ),
            (Some(bush), ..) => format!("Growth: {}%", (bush.growth * 100.0).round()),
            (_, Some(stack), ..) => {
                format!("{}: {} / {}", stack.kind.label(), stack.amount, STACK_CAP)
            }
            (_, _, Some(crop), ..) if to_harvest => format!(
                "{}: {}% - marked for harvest (yield {})",
                crop.kind.label(),
                (crop.growth * 100.0).round(),
                crop.yield_amount()
            ),
            (_, _, Some(crop), ..) => {
                format!("{}: {}%", crop.kind.label(), (crop.growth * 100.0).round())
            }
            (_, _, _, Some((status, skills, objective)), _, _) => format!(
                "Objective: {}\n{}\nHarvest: {}  Haul: {}  Farm: {}  Cut: {}  Build: {}",
                objective.label(),
                status.0,
                skills.harvest.letter(),
                skills.haul.letter(),
                skills.farm.letter(),
                skills.forestry.letter(),
                skills.build.letter()
            ),
            (_, _, _, _, Some((zone, region)), _) => {
                format!("{}: {} tiles", zone.kind.label(), region.len())
            }
            (.., Some(tree)) if tree.burnt && to_cut => "Charred - marked to clear".to_string(),
            (.., Some(tree)) if tree.burnt => "Charred - can be cleared".to_string(),
            (.., Some(tree)) if to_cut => format!(
                "Growth: {}% - marked to cut (yield {})",
                (tree.growth * 100.0).round(),
                tree.wood_yield()
            ),
            (.., Some(tree)) if tree.growth >= 1.0 => "Growth: 100% - mature".to_string(),
            (.., Some(tree)) => format!("Growth: {}% - sapling", (tree.growth * 100.0).round()),
            _ => String::new(),
        }
    };
    *hud = Visibility::Visible;
}

/// Show the action buttons the current selection supports.
#[allow(clippy::type_complexity)]
pub fn update_actions(
    selected: Res<SelectedEntity>,
    targets: Query<(
        Has<Harvestable>,
        Has<ToHarvest>,
        Has<Zone>,
        Option<&Tree>,
        Has<ToCut>,
        Has<Blueprint>,
        Has<Footprint>,
        Has<ToDemolish>,
    )>,
    mut buttons: Query<(&ActionButton, &mut Node)>,
) {
    let (harvestable, to_harvest, is_zone, tree, to_cut, is_blueprint, has_footprint, to_demolish) =
        selected
            .0
            .and_then(|entity| targets.get(entity).ok())
            .unwrap_or((false, false, false, None, false, false, false, false));
    let cuttable = tree.is_some_and(Tree::is_cuttable);
    // A built structure (any `BuildableKind`) carries `Footprint`; an
    // unbuilt ghost carries it too, so exclude `Blueprint` to demolish only
    // the finished thing, never a blueprint still under construction.
    let demolishable = has_footprint && !is_blueprint;
    for (button, mut node) in &mut buttons {
        let available = match button.action {
            TileAction::Harvest => harvestable && !to_harvest,
            TileAction::Cut => cuttable && !to_cut,
            TileAction::Demolish => demolishable && !to_demolish,
            TileAction::Cancel => to_harvest || to_cut || to_demolish || is_blueprint,
            TileAction::Delete | TileAction::Expand | TileAction::Shrink => is_zone,
        };
        node.display = if available {
            Display::Flex
        } else {
            Display::None
        };
    }
}

/// Hover/press feedback for the action buttons; a press runs the action on
/// the selected entity.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn run_actions(
    mut commands: Commands,
    mut selected: ResMut<SelectedEntity>,
    mut tool: ResMut<ActiveTool>,
    bushes: Query<(), (With<BerryBush>, With<Harvestable>)>,
    cuttable_trees: Query<&Tree>,
    zone_marker: Query<(), With<Zone>>,
    blueprint_marker: Query<(), With<Blueprint>>,
    built_structures: Query<(), (With<Footprint>, Without<Blueprint>)>,
    zone_tiles: Query<(Entity, &zones::ZoneTile)>,
    mut interactions: Query<
        (&ActionButton, &Interaction, &mut BackgroundColor),
        Changed<Interaction>,
    >,
) {
    for (button, interaction, mut background) in &mut interactions {
        let (base, hover, pressed) = button.action.colors();
        background.0 = match interaction {
            Interaction::Pressed => pressed,
            Interaction::Hovered => hover,
            Interaction::None => base,
        };
        if *interaction != Interaction::Pressed {
            continue;
        }
        match button.action {
            TileAction::Harvest => {
                if let Some(bush) = selected.0.filter(|bush| bushes.get(*bush).is_ok()) {
                    commands.entity(bush).insert(ToHarvest);
                }
            }
            TileAction::Cut => {
                let cuttable = |entity: &Entity| {
                    cuttable_trees
                        .get(*entity)
                        .is_ok_and(|tree| tree.is_cuttable())
                };
                if let Some(tree) = selected.0.filter(cuttable) {
                    commands.entity(tree).insert(ToCut);
                }
            }
            TileAction::Demolish => {
                if let Some(entity) = selected
                    .0
                    .filter(|entity| built_structures.get(*entity).is_ok())
                {
                    commands.entity(entity).insert(ToDemolish);
                }
            }
            // Cancelling is generic on purpose: strip every cancellable
            // designation the selection carries. Blueprints get a `ToCancel`
            // marker instead — `construction::cancel_blueprints` refunds the
            // delivered wood before despawning them.
            TileAction::Cancel => {
                if let Some(entity) = selected.0 {
                    commands
                        .entity(entity)
                        .remove::<(ToHarvest, ToCut, ToDemolish)>();
                    if blueprint_marker.get(entity).is_ok() {
                        commands.entity(entity).insert(ToCancel);
                    }
                }
            }
            TileAction::Delete => {
                if let Some(entity) = selected.0.filter(|entity| zone_marker.get(*entity).is_ok()) {
                    zones::despawn_zone(&mut commands, entity, &zone_tiles);
                    selected.0 = None;
                }
            }
            TileAction::Expand => {
                if let Some(entity) = selected.0.filter(|entity| zone_marker.get(*entity).is_ok()) {
                    tool.0 = Some(Tool::Expand(entity));
                }
            }
            TileAction::Shrink => {
                if let Some(entity) = selected.0.filter(|entity| zone_marker.get(*entity).is_ok()) {
                    tool.0 = Some(Tool::Shrink(entity));
                }
            }
        }
    }
}

/// Hover/press feedback for the general-action buttons; a press arms the
/// corresponding drag tool and collapses the category menu.
pub fn run_general_actions(
    mut tool: ResMut<ActiveTool>,
    mut open: ResMut<OpenCategory>,
    mut interactions: Query<
        (&GeneralActionButton, &Interaction, &mut BackgroundColor),
        Changed<Interaction>,
    >,
) {
    for (button, interaction, mut background) in &mut interactions {
        background.0 = match interaction {
            Interaction::Pressed => BUTTON_PRESSED,
            Interaction::Hovered => BUTTON_HOVER,
            Interaction::None => BUTTON_BACKGROUND,
        };
        if *interaction != Interaction::Pressed {
            continue;
        }
        match button.action {
            GeneralAction::CreateStockpile => tool.0 = Some(Tool::PlaceStockpile),
            GeneralAction::CreateGrowing => tool.0 = Some(Tool::PlaceGrowing),
            GeneralAction::CreateWall => tool.0 = Some(Tool::PlaceWall),
            GeneralAction::CreateDoor => tool.0 = Some(Tool::PlaceDoor),
            GeneralAction::CreateBed => tool.0 = Some(Tool::PlaceBed(BedOrientation::Horizontal)),
            GeneralAction::CreateTurbine => tool.0 = Some(Tool::PlaceTurbine),
            GeneralAction::CreateSolarPanel => tool.0 = Some(Tool::PlaceSolarPanel),
            GeneralAction::CreateLightpost => tool.0 = Some(Tool::PlaceLightpost),
            GeneralAction::CreateBattery => tool.0 = Some(Tool::PlaceBattery),
            GeneralAction::CreateCooler => tool.0 = Some(Tool::PlaceCooler),
            GeneralAction::RoofArea => tool.0 = Some(Tool::StampRoof),
            GeneralAction::NoRoofArea => tool.0 = Some(Tool::StampNoRoof),
            GeneralAction::ClearRoofArea => tool.0 = Some(Tool::StampClearRoof),
            GeneralAction::CutArea => tool.0 = Some(Tool::DesignateCut),
            GeneralAction::CancelCutArea => tool.0 = Some(Tool::CancelCut),
            GeneralAction::DemolishArea => tool.0 = Some(Tool::DesignateDemolish),
            GeneralAction::CancelDemolishArea => tool.0 = Some(Tool::CancelDemolish),
        }
        open.0 = None;
    }
}

/// Hover/press feedback for category buttons; a press opens that category's
/// sub-row, or closes the menu if it was already open.
pub fn run_category_buttons(
    mut open: ResMut<OpenCategory>,
    mut interactions: Query<
        (&CategoryButton, &Interaction, &mut BackgroundColor),
        Changed<Interaction>,
    >,
) {
    for (button, interaction, mut background) in &mut interactions {
        // The open category's color is owned by `sync_category_menu`; don't
        // fight it with hover/none coloring here.
        if open.0 != Some(button.category) {
            background.0 = match interaction {
                Interaction::Pressed => BUTTON_PRESSED,
                Interaction::Hovered => BUTTON_HOVER,
                Interaction::None => BUTTON_BACKGROUND,
            };
        }
        if *interaction != Interaction::Pressed {
            continue;
        }
        open.0 = if open.0 == Some(button.category) {
            None
        } else {
            Some(button.category)
        };
    }
}

/// Shows the open category's sub-row and hides the rest; tints the open
/// category's own button so the active group reads as selected.
pub fn sync_category_menu(
    open: Res<OpenCategory>,
    mut rows: Query<(&CategorySubRow, &mut Node)>,
    mut buttons: Query<(&CategoryButton, &mut BackgroundColor)>,
) {
    if !open.is_changed() {
        return;
    }
    for (row, mut node) in &mut rows {
        node.display = if open.0 == Some(row.category) {
            Display::Flex
        } else {
            Display::None
        };
    }
    for (button, mut background) in &mut buttons {
        background.0 = if open.0 == Some(button.category) {
            BUTTON_PRESSED
        } else {
            BUTTON_BACKGROUND
        };
    }
}

/// Escape or right-click collapses the category menu, mirroring how the same
/// inputs disarm the active drag tool (`zones::drag_zone_tool`).
pub fn close_category_menu(
    mut open: ResMut<OpenCategory>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
) {
    if open.0.is_some()
        && (keys.just_pressed(KeyCode::Escape) || buttons.just_pressed(MouseButton::Right))
    {
        open.0 = None;
    }
}

/// Outline the selected zone's true shape on the ground plane: for every
/// cell, draw the edge toward each neighbor that isn't also in the region.
/// Zones aren't necessarily rectangular (or even contiguous) any more, so a
/// bounding-box outline would be wrong — this hugs concave shapes, holes,
/// and disjoint islands correctly.
pub fn sync_zone_selection(
    selected: Res<SelectedEntity>,
    zones: Query<&ZoneRegion>,
    mut gizmos: Gizmos,
) {
    let Some(region) = selected.0.and_then(|entity| zones.get(entity).ok()) else {
        return;
    };
    let half = TILE_SIZE / 2.0;
    let y = ZONE_OUTLINE_HEIGHT;
    for cell in region.cells() {
        let center = map::grid_to_world(&cell);
        // World X grows with grid.x; world Z shrinks as grid.y grows (see
        // `map::grid_to_world`), so the "north" (grid.y + 1) neighbor sits
        // on the -Z edge of this tile, and "south" (grid.y - 1) on +Z.
        let nw = Vec3::new(center.x - half, y, center.z - half);
        let ne = Vec3::new(center.x + half, y, center.z - half);
        let se = Vec3::new(center.x + half, y, center.z + half);
        let sw = Vec3::new(center.x - half, y, center.z + half);
        let edges = [
            (GridCoords::new(cell.x + 1, cell.y), ne, se),
            (GridCoords::new(cell.x - 1, cell.y), nw, sw),
            (GridCoords::new(cell.x, cell.y + 1), nw, ne),
            (GridCoords::new(cell.x, cell.y - 1), sw, se),
        ];
        for (neighbor, a, b) in edges {
            if !region.contains(neighbor) {
                gizmos.line(a, b, ZONE_OUTLINE_COLOR);
            }
        }
    }
}
