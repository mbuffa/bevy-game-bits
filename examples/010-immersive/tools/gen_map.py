#!/usr/bin/env python3
"""Generate the 010-immersive warehouse map (Valve220 .map text).

Why this exists
---------------
`test.map` was hand-written, and SPEC.md records two bugs that came from doing
it by hand: a single brush is a *solid* box, not a hollow room; and every face
was wound backwards for bevy_trenchbroom's normal convention. This script makes
both impossible to get wrong twice -- rooms are declared as axis-aligned boxes
and the winding flip lives in exactly one place (`_face`).

bevy_trenchbroom's `BrushPlane::from_triangle` computes a face normal as
`(p3 - p1) x (p2 - p1)` -- the opposite of the textbook `(p2-p1) x (p3-p1)`.
The six entries in `_BOX_FACES` were each checked against a face of the old
`test.map` whose outward normal is known, so a box declared with `mins < maxs`
on every axis comes out solid with all normals facing out.

Output
------
Two files, from one box list:

    assets/maps/immersive/warehouse.map             -- the runtime map. No
        Quake `light` entity, but a bank of `light_fixture` point entities
        (real-time SpotLights, built by lights.rs) and one `func_light_switch`
        (Phase 9). The main bank loads *on* (Phase 12).
    assets/maps/immersive/warehouse_bsp_source.map  -- adds a Quake `light`
        at each fixture origin and an `info_player_start` for the ericw-tools
        compile (`make bsp`). Not the default; not verified unless built.

Run from anywhere:  python3 examples/010-immersive/tools/gen_map.py
Deterministic: re-running produces byte-identical files.

Geometry (TrenchBroom units, +Z up, +Y north; 39.37008 u/m)
----------------------------------------------------------
Phase 21 revamp (iteration 2) -- a warehouse STORAGE hall with real forklift-
height PALLET RACKING. Interior 1920 x 1344 x 432 u (48.8 x 34.1 x 10.97 m).
FOUR long north-south racking runs (x-centres -448/-180/120/400, ~88 u wide,
y 8..520), each 3 pallet-level DECK slabs (tops z 64/192/304) on iron uprights,
with ~4.6-5.8 m aisles between them and a north cross-aisle. Only the x -448 run
is CLIMBABLE: it drops the z-64 level and LADDER is bolted to its south
end-frame's outer face, climbing to the z-192 slab, where the wooden crate
(lockpick) + crowbar sit -- throw the crate off to break it. A person-sized roof-access
PLATFORM + its own func_ladder (LADDER2) is tucked in the NW corner (future roof
hatch).

The OFFICE is boxed into the SE corner (x 656..960, y -672..-416, z 0..144,
flat standable roof). Its west wall has the one LOCKED `prop_door` (the lockpick
target); the LOADING wall (east) has a matching UNLOCKED exterior door on the
same y line (_OFF_DOOR_CY), plus two large sealed truck-door panels (TRUCK_*,
DECK-textured steel; migrate one to a `func_door` roll-up later). East of the
loading wall is the open truck YARD (x 976..1776, `skip` lid for `make bsp`,
real sky later) -- reached only through the office's two doors.

Two `prop_crate` props on the spawn line drive the autopilot (PINNED); the old
BIG_CRATE + 3 stacking crates are floor clutter in the open south staging strip
(kept for the weight gate and the liftable-crate count).

Lighting: a 4x3 ceiling grid (z 432) + 1 shadow-caster over the crate bay + 1
lamp per rack aisle, all "main_lights" (*on* at load); 4 always-on "night" wall
brackets; 2 "office" + 4 "yard" lamps, always on; a `func_light_switch` beside
the office door (z 64, floor-reachable) toggles the whole "main_lights" bank.
"""

import math
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
MAP_DIR = os.path.normpath(os.path.join(HERE, "..", "..", "..", "assets", "maps", "immersive"))

# --- box -> six faces, winding matched to bevy_trenchbroom ------------------
#
# Each entry: (axis, sign, (p1, p2, p3)) where every point is a triple of
# indices into (x0, y0, z0, x1, y1, z1): 0/3 = x min/max, 1/4 = y, 2/5 = z.
_BOX_FACES = [
    # -Z (bottom): p1=(x0,y0,z0) p2=(x1,y1,z0) p3=(x0,y1,z0)
    ("z", -1, ((0, 1, 2), (3, 4, 2), (0, 4, 2))),
    # +Z (top): p1=(x0,y0,z1) p2=(x0,y1,z1) p3=(x1,y1,z1)
    ("z", 1, ((0, 1, 5), (0, 4, 5), (3, 4, 5))),
    # -X (west): p1=(x0,y0,z0) p2=(x0,y1,z1) p3=(x0,y0,z1)
    ("x", -1, ((0, 1, 2), (0, 4, 5), (0, 1, 5))),
    # +X (east): p1=(x1,y0,z0) p2=(x1,y0,z1) p3=(x1,y1,z1)
    ("x", 1, ((3, 1, 2), (3, 1, 5), (3, 4, 5))),
    # -Y (south): p1=(x0,y0,z0) p2=(x0,y0,z1) p3=(x1,y0,z1)
    ("y", -1, ((0, 1, 2), (0, 1, 5), (3, 1, 5))),
    # +Y (north): p1=(x0,y1,z0) p2=(x1,y1,z1) p3=(x0,y1,z1)
    ("y", 1, ((0, 4, 2), (3, 4, 5), (0, 4, 5))),
]

# UV axes per face normal axis, copied from test.map's convention.
_UV = {
    "z": ("[ 1 0 0 0 ]", "[ 0 -1 0 0 ]"),
    "x": ("[ 0 1 0 0 ]", "[ 0 0 -1 0 ]"),
    "y": ("[ 1 0 0 0 ]", "[ 0 0 -1 0 ]"),
}
_SCALE = "0 0.25 0.25"  # rotation xscale yscale


def _pt(coords, idx):
    return (coords[idx[0]], coords[idx[1]], coords[idx[2]])


def _fmt(p):
    return "( %s %s %s )" % tuple(int(v) if float(v).is_integer() else v for v in p)


# Every box_brush call records its AABB here so `_check_overlaps` can flag two
# structural brushes that deeply share a *volume* (the "abut, don't overlap"
# rule — the round-2 corridor z-fight). The 16 u shell brushes legitimately
# interpenetrate each other and the floor/ceiling by exactly one shell
# thickness at every corner, so an overlap whose smallest dimension is <= the
# shell thickness is a seam, not a bug, and is ignored.
_BOXES = []
_SHELL = 16


def _check_overlaps():
    bad = []
    for i in range(len(_BOXES)):
        (amn, amx, at) = _BOXES[i]
        for j in range(i + 1, len(_BOXES)):
            (bmn, bmx, bt) = _BOXES[j]
            ov = [min(amx[k], bmx[k]) - max(amn[k], bmn[k]) for k in range(3)]
            if min(ov) > _SHELL:
                bad.append((amn, amx, at, bmn, bmx, bt, tuple(round(o, 2) for o in ov)))
    return bad


def box_brush(mins, maxs, tex, tex_top=None, tex_bottom=None):
    """A solid axis-aligned box. `tex` is the default; `tex_top`/`tex_bottom`
    override the +Z / -Z faces."""
    x0, y0, z0 = mins
    x1, y1, z1 = maxs
    assert x0 < x1 and y0 < y1 and z0 < z1, (mins, maxs)
    _BOXES.append((tuple(mins), tuple(maxs), tex))
    coords = (x0, y0, z0, x1, y1, z1)
    lines = ["{"]
    for axis, sign, tri in _BOX_FACES:
        face_tex = tex
        if axis == "z":
            face_tex = tex_top if sign == 1 and tex_top else (
                tex_bottom if sign == -1 and tex_bottom else tex)
        p1, p2, p3 = (_pt(coords, i) for i in tri)
        u, v = _UV[axis]
        lines.append("%s %s %s %s %s %s %s" % (_fmt(p1), _fmt(p2), _fmt(p3), face_tex, u, v, _SCALE))
    lines.append("}")
    return "\n".join(lines)


# --- the warehouse --------------------------------------------------------

# Surface families, one texture name each (patterns generated by
# tools/gen_textures.py). FLOOR = light-grey concrete (floor *and* ceiling --
# a brick roof looks wrong), WALL = dark-red brick, DECK = dark-grey iron --
# the platforms *and* the columns that hold them up, so the load-bearing
# structure reads as one material.
FLOOR = "dev_floor_concrete"
WALL = "dev_wall_brick"
DECK = "dev_deck_iron"
# `skip` is in bevy_trenchbroom's default `auto_remove_textures`: the brush
# renders nothing but still contributes a convex collider. ladder.rs builds the
# visible rails-and-rungs mesh itself from this brush's AABB.
LADDER_TEX = "skip"

# Storage hall interior: x ±HALL_HX, y ±HALL_HY, z 0..CEILING_Z. 16 u shell.
#
# Phase 21 revamp (iteration 2): the hall is a warehouse *storage room* with
# real forklift-height PALLET RACKING (multi-level, not shop shelving you climb).
# Iteration 1's stepped-shelf staircase is gone; the racking is unclimbable, so
# a second `func_ladder` is bolted to the one bay that holds the loot. The
# ceiling went back up to ~11 m and the footprint grew ~50 % so there's an aisle
# around every rack run and a clear gap to the office. The east (LOADING) wall
# carries the office man-door + two sealed truck-door panels. See SPEC.md Ph 21.
#
# Pinned so `config::AUTOPILOT_SCRIPT` / `IMMERSIVE_AUTOPILOT=crates` need no
# numeric change: PLAYER_SPAWN, the z-192 crest slab the rack ladder climbs to,
# and AUTOPILOT_CRATES. LADDER_BRUSH itself is *not* pinned any more (Phase 21
# iteration 4 re-sited it onto the end frame's outer face) — only its comment
# in config.rs moved, not the script's timing.
DOORWAY_HW = 22    # man-door half-width (u); DOOR_WIDTH is ~1.12 m ≈ 44 u
DOORWAY_H = 90     # man-door opening height (u) ≈ 2.3 m
HALL_HX = 960     # hall interior half-width (u) ≈ 24.4 m  -> 48.8 m across
HALL_HY = 672     # hall interior half-depth (u) ≈ 17.1 m  -> 34.1 m deep
CEILING_Z = 432   # hall ceiling underside (u) ≈ 10.97 m
_OX, _OY = HALL_HX + 16, HALL_HY + 16   # outer (shell) half-extents
TRUCK_HW = 88     # truck-door half-width (u); ~4.47 m opening
TRUCK_H = 160     # truck-door height (u) ≈ 4.06 m
_TRUCK_Y = (120, 440)  # centre y of each truck-door opening in the loading wall
_OFF_DOOR_CY = -544    # centre y of the office man-door (both leaves line up on it)

# The office man-door hole, shared by the locked west leaf and the exterior
# east leaf: y _OFF_DOOR_CY ± DOORWAY_HW, z 0..DOORWAY_H.
_ODY0, _ODY1 = _OFF_DOOR_CY - DOORWAY_HW, _OFF_DOOR_CY + DOORWAY_HW


def _wall_with_holes(x0, x1, y_lo, y_hi, z_top, holes):
    """A wall panel running along Y (x0..x1, y_lo..y_hi, z -16..z_top) split
    around a list of `(y0, y1, hole_z_top)` openings. Solid below/around each
    hole, a lintel brush from `hole_z_top` to `z_top` over it."""
    holes = sorted(holes)
    out, y = [], y_lo
    for hy0, hy1, hz in holes:
        if hy0 > y:
            out.append(box_brush((x0, y, -16), (x1, hy0, z_top), WALL))
        out.append(box_brush((x0, hy0, hz), (x1, hy1, z_top), WALL))  # lintel
        y = hy1
    if y < y_hi:
        out.append(box_brush((x0, y, -16), (x1, y_hi, z_top), WALL))
    return out


WORLDSPAWN = [
    # floor / ceiling -- both concrete
    box_brush((-_OX, -_OY, -16), (_OX, _OY, 0), FLOOR, tex_top=FLOOR),
    box_brush((-_OX, -_OY, CEILING_Z), (_OX, _OY, CEILING_Z + 16), FLOOR),
    # walls: west, south, north
    box_brush((-_OX, -_OY, -16), (-HALL_HX, _OY, CEILING_Z + 16), WALL),
    box_brush((-HALL_HX, -_OY, -16), (HALL_HX, -HALL_HY, CEILING_Z + 16), WALL),
    box_brush((-HALL_HX, HALL_HY, -16), (HALL_HX, _OY, CEILING_Z + 16), WALL),
]

# East / loading wall: the office man-door hole + the two truck-door holes.
WORLDSPAWN += _wall_with_holes(HALL_HX, _OX, -_OY, _OY, CEILING_Z + 16, [
    (_ODY0, _ODY1, DOORWAY_H),
    (_TRUCK_Y[0] - TRUCK_HW, _TRUCK_Y[0] + TRUCK_HW, TRUCK_H),
    (_TRUCK_Y[1] - TRUCK_HW, _TRUCK_Y[1] + TRUCK_HW, TRUCK_H),
])

# --- pallet racking -------------------------------------------------------
#
# Four long north-south racking runs — the classic warehouse grid. Each run: 3
# pallet-level `DECK` slabs (tops at z 64 / 192 / 304 — 1.6 / 4.9 / 7.7 m) on
# 16 u iron uprights (end + mid frames). Slabs are inset 16 u in x so they abut
# the uprights instead of z-fighting through them. Runs are ~88 u wide, span
# y 8..520 (open south strip for the spawn / crate-ladder walk / autopilot
# crates; north cross-aisle y 520..HALL_HY). Aisle centres ≈ x -314 / -30 / 260.
#
# The x -448 run is the only CLIMBABLE one: it drops the z-64 floor level (so
# nothing sits in the ladder's drop column) and LADDER_BRUSH is bolted to its
# south end-frame's outer face, climbing to the z-192 slab, where the wooden
# crate + crowbar sit.
_RACK_TOPS = (64, 192, 304)
_RACK_HW = 44          # run half-width (x)
_RACK_Y = (8, 520)     # run extent (y)

# Shared `func_ladder` dimensions (both LADDER_BRUSH and LADDER2_BRUSH): a
# believable 24 u (0.61 m) rail spacing, a generous grab-volume depth in y
# (never drawn — only the brush *centre* is visual, LADDER_VISUAL_DEPTH deep),
# and an inset so that centre lands right on the face it's bolted to.
_LADDER_HW = 12    # half-width in x -> 24 u between the rails
_LADDER_D = 40     # grab-volume depth in y; aim generosity only
_LADDER_INSET = 2  # ~half the 0.10 m visual depth, so the rungs abut the face


def _pallet_run(cx, tops=_RACK_TOPS):
    y0, y1 = _RACK_Y
    x0, x1 = cx - _RACK_HW, cx + _RACK_HW
    sx0, sx1 = x0 + 16, x1 - 16                       # slabs inset from the uprights
    out = [box_brush((sx0, y0, t - 16), (sx1, y1, t), DECK, tex_top=DECK) for t in tops]
    up_top = max(tops) + 16
    ymid = (y0 + y1) // 2 - 8
    for uy in (y0, ymid, y1 - 16):
        for ux in (x0, x1 - 16):
            out.append(box_brush((ux, uy, 0), (ux + 16, uy + 16, up_top), DECK))
    return out


RACK_CRATE_CX = -448
RACKS = (
    _pallet_run(RACK_CRATE_CX, tops=(192, 304))   # climbable — no z-64 slab in the drop column
    + _pallet_run(-180)
    + _pallet_run(120)
    + _pallet_run(400)
)

# --- roof-access platform (NW corner) -----------------------------------
#
# Person-sized (88 u ≈ 2.2 m square) landing tucked against the west + north
# walls, in the north cross-aisle. Its own `func_ladder` (LADDER2_*) is bolted
# to its south face. Roof access itself is still future work — a third
# func_ladder through a ceiling hole above this platform.
_PLATFORM_CX = -916          # (-960 + -872) / 2
_PLATFORM_SOUTH_Y = 584
PLATFORM = [
    box_brush((-960, 584, 176), (-872, 672, 192), DECK, tex_top=DECK),
    box_brush((-960, 584, 0), (-944, 600, 176), DECK),   # SW upright
    box_brush((-888, 584, 0), (-872, 600, 176), DECK),   # SE upright
]
_LADDER2_CY = _PLATFORM_SOUTH_Y - _LADDER_INSET
LADDER2_BRUSH = box_brush(
    (_PLATFORM_CX - _LADDER_HW, _LADDER2_CY - _LADDER_D // 2, 0),
    (_PLATFORM_CX + _LADDER_HW, _LADDER2_CY + _LADDER_D // 2, 208),
    LADDER_TEX,
)
LADDER2_ENTITY = {"classname": "func_ladder", "face_yaw": "90", "prompt": "Climb ladder"}

# --- management office (SE corner) ----------------------------------------
#
# Interior x 656..960, y -672..-416, z 0..144 (7.7 x 6.5 x 3.66 m). Its south
# and east sides are the hall's own shell. The WEST wall carries the locked
# `prop_door` (the lockpick target); the exterior man-door in the loading wall
# lines up with it on y _OFF_DOOR_CY, so once picked you walk storage -> office
# -> yard straight. Flat roof z 144..160 — standable.
OFFICE = [
    box_brush((656, -416, -16), (960, -400, 144), WALL),                  # north wall
    box_brush((640, -_OY, 144), (960, -400, 160), FLOOR, tex_top=FLOOR),  # roof slab
] + [
    box_brush((640, y, -16), (656, ny, 144), WALL)                        # west wall, split
    for y, ny in ((-_OY, _ODY0), (_ODY1, -400))
] + [
    box_brush((640, _ODY0, DOORWAY_H), (656, _ODY1, 144), WALL),          # door lintel
]

# --- truck-door panels (sealed) ------------------------------------------
#
# Each truck-door opening is filled by a DECK-textured steel panel recessed 4 u
# from the interior face — reads as a roll-up shutter set into the brick, not
# painted wall. Solid, not openable: that is what "locked" means for now.
# Migration: to make one a working roll-up later, lift its panel out of
# WORLDSPAWN into a `func_door` entity with `angle -1`. `FuncDoor` is declared
# and `door.rs` drives it, but no instance has ever been placed and its `angle`
# field is a latent bug (SPEC.md) — its own phase.
TRUCK_PANELS = [
    box_brush((964, cy - TRUCK_HW, 0), (976, cy + TRUCK_HW, TRUCK_H), DECK)
    for cy in _TRUCK_Y
]

# --- truck yard (replaces corridor -> bay -> yard) ------------------------
#
# One open apron east of the loading wall. Its west boundary IS the hall's east
# shell — nothing new at the loading wall; it connects to the building only
# through the office's exterior door. The `skip` lid renders nothing at runtime
# but seals the volume so `make bsp` (ericw-tools qbsp) doesn't leak on open sky
# (a real sun + skybox is a later phase).
_YARD_E = 1776
YARD = [
    box_brush((_OX, -_OY, -16), (_YARD_E, _OY, 0), FLOOR, tex_top=FLOOR),
    box_brush((_OX, -_OY, 360), (_YARD_E, _OY, 376), "skip"),          # lid
    box_brush((_YARD_E - 16, -_OY, -16), (_YARD_E, _OY, 376), WALL),   # east
    box_brush((_OX, -_OY, -16), (_YARD_E, -HALL_HY, 376), WALL),       # south
    box_brush((_OX, HALL_HY, -16), (_YARD_E, _OY, 376), WALL),         # north
]

WORLDSPAWN += RACKS + PLATFORM + OFFICE + TRUCK_PANELS + YARD

# Crate-rack access ladder: bolted to the OUTER (south) face of the x -448
# racking run's south end frame, from the floor to 16 u above the z-192 pallet
# slab. `skip`-textured -> invisible; ladder.rs turns it into a Sensor +
# builds the rails-and-rungs mesh + the *solid* thin collider that actually
# stops you. The visual plane is the brush *centre* (ladder.rs builds the
# mesh at `aabb.center()`, only LADDER_VISUAL_DEPTH deep), so the centre —
# not the near face — is what has to sit on `_RACK_Y[0]`.
_LADDER_CY = _RACK_Y[0] - _LADDER_INSET
LADDER_BRUSH = box_brush(
    (RACK_CRATE_CX - _LADDER_HW, _LADDER_CY - _LADDER_D // 2, 0),
    (RACK_CRATE_CX + _LADDER_HW, _LADDER_CY + _LADDER_D // 2, 208),
    LADDER_TEX,
)

# --- crates ---------------------------------------------------------------
#
# Carryable metal props (`prop_crate`, see classes.rs / carry.rs). Three groups:
#
#  * AUTOPILOT_CRATES -- on the line *behind* the spawn (away from the ladder),
#    so `IMMERSIVE_AUTOPILOT=crates` reaches them by turning 180 deg and walking
#    forward, and the `IMMERSIVE_AUTOPILOT=1` ladder walk (which heads the other
#    way) never touches them. One normal, one over-`CARRY_MAX_MASS`. PINNED.
#  * BIG_CRATE + STACKING_CRATES -- floor clutter in the open south staging
#    strip (clear of all 4 rack runs and the office). BIG_CRATE is still the
#    over-`CARRY_MAX_MASS` weight gate; the three 0.8 m crates keep `crates(n=)`
#    in telemetry at 4 liftable at rest (autopilot normal + 3).
#
# `size` is the cube edge in metres (`config::CRATE_SIZE` default 0.8); origin
# is the cube centre, so z = size/2 * scale rests it on the floor (top z 0).
# TB units per metre (== config::TB_SCALE). NB: deliberately *not* `_SCALE` —
# that name belongs to `box_brush`'s Valve220 "rotation xscale yscale" string,
# and any brush built after this line (the light switch) would emit this float
# in its place and break the parser.
_UPM = 39.37008


def _crate(x, y, mass=25, size=0.8, prompt="Hold crate"):
    return {"classname": "prop_crate",
            "origin": "%d %d %d" % (x, y, round(size * _UPM / 2)),
            "prompt": prompt, "mass": str(mass), "size": str(size)}


# Player spawns at y -256 facing +Y (the ladder). These sit the *other* way,
# toward the south wall (y -448), clear of the ladder walk.
AUTOPILOT_CRATES = [
    _crate(-448, -330),                                  # normal: grab / place / throw
    _crate(-448, -390, mass=400, prompt="Crate (too heavy)"),  # weight gate
]

# Open floor in the south staging strip (y < 8, south of every rack run; well
# west of the office at x >= 640).
BIG_CRATE = _crate(240, -520, mass=800, size=1.6, prompt="Heavy crate")
STACKING_CRATES = [
    _crate(140, -560),
    _crate(240, -600),
    _crate(340, -560),
]

# --- props --------------------------------------------------------------------
#
# Two hinged doors (`prop_door`, door.rs), the breakable wooden crate
# (`prop_wood_crate`, breakable.rs) and the crowbar `item_pickup` (items.rs).
# `prop_door` origin is the HINGE at floor level; `prop_wood_crate` and
# `item_pickup` origins are the model centre, so z = size/2 (+ z_base) rests
# them on the surface.


def _door(x, y, face_yaw, locked=0, prompt="Open door"):
    # `x` sits the ~0.08 m leaf just inside the near wall face (not the wall
    # mid-line) so you don't see daylight around a thin leaf in a thick brush.
    # `y` is the HINGE — on a jamb, at hole-centre + DOORWAY_HW. `width`/`height`
    # are DERIVED from the (fixed-size) man-door hole so the leaf and the
    # opening can't drift apart; `door.rs` then laps the leaf `DOOR_REBATE` over
    # the frame on top of that.
    return {"classname": "prop_door",
            "origin": "%d %d 0" % (x, y),
            "face_yaw": str(face_yaw), "locked": str(locked), "prompt": prompt,
            "width": "%.4f" % (2 * DOORWAY_HW / _UPM),
            "height": "%.4f" % (DOORWAY_H / _UPM)}


def _wood_crate(x, y, contains, size=0.6, z_base=0,
                prompt="Wooden crate — throw it to break it open"):
    # origin is the cube centre; z_base is the surface it rests on (0 = floor,
    # 192 = the crate rack's z-192 pallet slab).
    return {"classname": "prop_wood_crate",
            "origin": "%d %d %d" % (x, y, z_base + round(size * _UPM / 2)),
            "size": str(size), "contains": contains, "prompt": prompt}


def _pickup(key, x, y, z):
    return {"classname": "item_pickup", "origin": "%d %d %d" % (x, y, z), "item": key}


# Placement:
#  * the LOCKED `prop_door` fills the office's west-wall man-door hole (hinge on
#    the north jamb at _OFF_DOOR_CY + DOORWAY_HW, `face_yaw 0` so the closed leaf
#    lies across the opening and swings east into the office, away from a player
#    approaching from the storage floor). This is the only locked door — pick it
#    and step through the office to its exterior door.
#  * the UNLOCKED exterior `prop_door` fills the matching hole in the loading
#    wall on the same y line, swinging out into the yard.
#  * the wooden crate (lockpick) + the crowbar sit on the crate rack's z-192
#    pallet slab, reached by its access ladder. Throw the crate off the slab
#    (~4.9 m) onto the concrete to break it — or crowbar it in place.
#
# No loose "lockpick" pickup — it only exists once the crate breaks.
_CRATE_SLAB_Z = 192            # the crate rack's climbable pallet slab top
PROPS = [
    _door(643, _OFF_DOOR_CY + DOORWAY_HW, face_yaw=0, locked=1, prompt="Open door"),
    _door(963, _OFF_DOOR_CY + DOORWAY_HW, face_yaw=0, locked=0, prompt="Open door"),
    _wood_crate(RACK_CRATE_CX, 150, contains="lockpick", z_base=_CRATE_SLAB_Z),
    _pickup("crowbar", RACK_CRATE_CX, 210, _CRATE_SLAB_Z + 18),
]

# --- lighting -----------------------------------------------------------------
#
# The warehouse loads LIT: `GlobalAmbientLight` at `config::AMBIENT_LIT`, no
# sun, the `main_lights` bank *on*. The `func_light_switch` (now on the storage
# floor beside the office door, hand height) still toggles the whole
# `main_lights` circuit. Four wall brackets on `targetname "night"` — which no
# switch drives — stay lit: the always-on aid if the mains are flipped off. The
# office and yard have their own always-on circuits.
#
# `light_fixture` is a point class (classes.rs): origin is the lamp's mount
# point (a ceiling / platform-underside / wall surface — `lights.rs` embeds the
# connector 0.02 m into it), `targetname` its circuit, `aim` ("down" default,
# or a compass word for a raked wall bracket), and every SpotLight knob
# (intensity, range, cone_deg, color, shadows) defaults from `config::LAMP_*`.
#
# NB: the room is ~2.25x iteration 1's floor area and the ceiling is at z 432,
# so the bank grew — a 4x3 ceiling grid + one lamp per rack aisle. Intensity is
# still `config::LAMP_INTENSITY`; expect a screenshot retune (SPEC.md Phase 21).


def _lamp(x, y, z, targetname, **over):
    d = {"classname": "light_fixture", "origin": "%d %d %d" % (x, y, z),
         "targetname": targetname}
    for k, v in over.items():
        d[k] = str(v)
    return d


# Ceiling bank: a 4 x 3 grid seated on the z 432 ceiling. `config.LAMP_RANGE`
# (20 m) keeps every lamp's cone inside the hall on its own, EXCEPT the SE
# corner of the grid — those three sit directly over the office's roof slab,
# so no range/cone value keeps their light out of the room below; `shadows=1`
# is the only fix (see `config::LAMP_SHADOWS`; `_check_leaks()` below is what
# actually enforces this). Same reasoning for the office-door/switch-corner
# lamp, which is close enough to the office's west wall to punch through it.
# One further shadow-caster over the crate rack's south bay, kept from
# iteration 1, so the player on the ladder / a growing crate stack reads as a
# shadow. All `start_on=1`.
_SE_SHADOW_LAMPS = {(210, -420), (620, -420), (620, 60), (620, 500)}
CEILING_LAMPS = [
    _lamp(
        x, y, CEILING_Z, "main_lights", start_on=1, cone_deg=110,
        **({"shadows": 1} if (x, y) in _SE_SHADOW_LAMPS else {}),
    )
    for x in (-620, -210, 210, 620)
    for y in (-420, 60, 500)
] + [
    _lamp(-448, 100, CEILING_Z, "main_lights", start_on=1, shadows=1, cone_deg=110),
    _lamp(560, -560, CEILING_Z, "main_lights", start_on=1, shadows=1, cone_deg=120),  # office door + switch corner
]

# One lamp hung on the rack uprights (~z 320) down each of the three aisles
# between the four runs, so the racking doesn't throw black canyons.
AISLE_LAMPS = [
    _lamp(cx, 264, 320, "main_lights", start_on=1, cone_deg=120)
    for cx in (-314, -30, 260)
]

# Night brackets: two on the south wall, raked north toward the spawn; two on
# the north wall, raked south. z ~360, near the ceiling. `targetname "night"` —
# no switch drives it, so they stay lit if the mains are flipped off (this is
# what you see in the dark state — the reported light-leak screenshot was
# taken with the mains off, which is why these and the yard floods below
# mattered most). Their cone/range predates `config.LAMP_RANGE`'s 20 m default
# and reaches clean across the hall to the spawn — kept exactly as-is rather
# than trimmed (that would gut the dark-state look), so each carries an
# explicit `range=30` override plus `shadows=1` to stop that long reach from
# also lighting the office/yard on the other side of the hall's walls.
WALL_NIGHT_LAMPS = [
    _lamp(-360, -HALL_HY, 360, "night", aim="north", intensity=340000,
          color="255 210 160", cone_deg=130, range=30, shadows=1, start_on=1),
    _lamp(360, -HALL_HY, 360, "night", aim="north", intensity=340000,
          color="255 210 160", cone_deg=130, range=30, shadows=1, start_on=1),
    _lamp(-360, HALL_HY, 360, "night", aim="south", intensity=300000,
          color="255 210 160", cone_deg=120, range=30, shadows=1, start_on=1),
    _lamp(360, HALL_HY, 360, "night", aim="south", intensity=300000,
          color="255 210 160", cone_deg=120, range=30, shadows=1, start_on=1),
]

# Office: two `"down"` ceiling lamps on its own always-on circuit. Brighter than
# the default — the low (z 144) ceiling puts them close, but the room read dark.
# `shadows=1` on both: at cone_deg=115 (~57.5° half-angle) each cone's floor
# footprint is wider than the office is deep, so without a shadow map to
# occlude it the cone punches straight through the west wall and lit the hall
# floor beyond the switch corner (SPEC.md Phase 21 iter 2 lighting bug).
OFFICE_LAMPS = [
    _lamp(740, -470, 144, "office", start_on=1, cone_deg=115, intensity=2600000, shadows=1),
    _lamp(880, -600, 144, "office", start_on=1, cone_deg=115, intensity=2600000, shadows=1),
]

# Yard: four cool floods hung under the `skip` lid, own always-on circuit.
# The west pair sits at x 1350, not right against the loading wall (x 976) —
# at the old x 1150 their cone reached back through the wall and lit the hall
# floor beyond it (the reported light-leak screenshot). Moving them out
# clears the leak *and* improves yard floor coverage, since they were
# wasting half their cone on the wall.
YARD_LAMPS = [
    _lamp(x, y, 340, "yard", start_on=1, color="220 230 255")
    for x in (1350, 1550)
    for y in (-320, 320)
]

LAMPS = CEILING_LAMPS + AISLE_LAMPS + WALL_NIGHT_LAMPS + OFFICE_LAMPS + YARD_LAMPS

# The light switch. A 12 x 40 x 48 u `skip` box standing 12 u proud of the
# office's west wall (hall face x 640), just north of the office door, centred
# at z 64 — ~1.63 m, hand height, reachable from the storage floor. `skip` is
# invisible but still yields a collider (the aim target); lights.rs draws a
# fixed-size lit panel + red PointLight from the AABB (the func_ladder
# precedent). It still targets `main_lights` and still starts on.
SWITCH_BRUSH = box_brush((628, -500, 40), (640, -460, 88), "skip")
SWITCH_ENTITY = {"classname": "func_light_switch", "target": "main_lights",
                 "prompt": "Turn off the lights", "start_on": "1"}

PLAYER_SPAWN = {"classname": "player_spawn", "origin": "-448 -256 48", "angle": "90"}
# NB: no "angle" key — bevy_trenchbroom would read it as a brush rotation and
# throw the ladder across the room. `face_yaw` is plain data ladder.rs reads.
LADDER_ENTITY = {"classname": "func_ladder", "face_yaw": "90", "prompt": "Climb ladder"}
# For the .bsp compile only: a matching Quake `light` at each lamp's origin so
# the baked path agrees with the dynamic one. Not verified unless `make bsp`
# is run — the default example loads warehouse.map.
BSP_LIGHTS = [
    {"classname": "light", "origin": lamp["origin"], "light": "400"}
    for lamp in LAMPS
]
# qbsp's leak-fill occupant; classname is literal. Our player_spawn coexists.
INFO_PLAYER_START = {"classname": "info_player_start", "origin": "-448 -256 48", "angle": "90"}

HEADER = """\
// 010-immersive warehouse map -- GENERATED by
// examples/010-immersive/tools/gen_map.py, do not edit by hand.
//
// A warehouse STORAGE hall (1920 x 1344 x 432 u ~= 48 x 34 x 11 m) with four
// long north-south PALLET RACKING runs and aisles between them. You spawn on
// the floor facing the one climbable run: an iron access ladder up its south
// end-frame to a z-192 pallet slab holding a wooden crate (lockpick inside)
// and a crowbar -- throw the crate off to break it open. A light switch by the
// office door toggles the main bank. A small roof-access platform + ladder is
// tucked in the NW corner. The east (loading) wall has two large sealed
// truck-door panels and one man-door: the office's exterior door. A LOCKED
// door in the office's west wall is what the lockpick opens; through the
// management office (SE corner) and out its back door is the open truck yard.
// See SPEC.md Phases 7-21.
"""


def _entity(props, brush=None):
    lines = ["{"]
    for k, v in props.items():
        lines.append('"%s" "%s"' % (k, v))
    if brush is not None:
        lines.append(brush)
    lines.append("}")
    return "\n".join(lines)


def build(with_light):
    parts = [HEADER.rstrip("\n")]

    world_props = ['{', '"classname" "worldspawn"']
    for b in WORLDSPAWN:
        world_props.append(b)
    world_props.append("}")
    parts.append("\n".join(world_props))

    parts.append(_entity(LADDER_ENTITY, LADDER_BRUSH))
    parts.append(_entity(LADDER2_ENTITY, LADDER2_BRUSH))
    for crate in AUTOPILOT_CRATES + [BIG_CRATE] + STACKING_CRATES:
        parts.append(_entity(crate))
    for prop in PROPS:
        parts.append(_entity(prop))
    for lamp in LAMPS:
        parts.append(_entity(lamp))
    parts.append(_entity(SWITCH_ENTITY, SWITCH_BRUSH))
    parts.append(_entity(PLAYER_SPAWN))
    if with_light:
        parts.append(_entity(INFO_PLAYER_START))
        for light in BSP_LIGHTS:
            parts.append(_entity(light))
    return "\n".join(parts) + "\n"


# Default `SpotLight` knobs a `_lamp()` call didn't override -- mirrors
# `config::LAMP_CONE_DEG` / `config::LAMP_RANGE`. Kept in sync by hand, same as
# `_UPM` == `config::TB_SCALE`.
_DEFAULT_CONE_DEG = 95.0
_DEFAULT_RANGE_M = 20.0

# The three sealed rooms a lamp's light must stay inside of unless it casts a
# shadow (`_check_leaks`, below). Interior AABBs in TB units -- OFFICE mirrors
# the `OFFICE` brush list above (x 656..960, its west wall's east face to the
# loading wall; y -_OY.._ODY0-ish down to -400) and must be kept in sync with
# it by hand; YARD mirrors the `YARD` brush list.
_LEAK_ROOMS = {
    "HALL": (-HALL_HX, HALL_HX, -HALL_HY, HALL_HY, 0, CEILING_Z),
    # Carved out of the hall's own footprint -- tested before HALL below, or
    # every office lamp would also register as "in HALL".
    "OFFICE": (656, 960, -672, -416, 0, 144),
    "YARD": (_OX, _YARD_E, -_OY, _OY, 0, 360),
}
_LEAK_AIM = {"down": (0, 0, -1), "north": (0, 1, 0), "south": (0, -1, 0),
             "east": (1, 0, 0), "west": (-1, 0, 0)}


def _leak_room_of(p):
    for name in ("OFFICE", "YARD", "HALL"):
        x0, x1, y0, y1, z0, z1 = _LEAK_ROOMS[name]
        if x0 <= p[0] <= x1 and y0 <= p[1] <= y1 and z0 <= p[2] <= z1:
            return name
    return None


def _leak_samples(name, n=20):
    x0, x1, y0, y1, z0, z1 = _LEAK_ROOMS[name]
    pts = []
    for i in range(n + 1):
        x = x0 + (x1 - x0) * i / n
        for j in range(n + 1):
            y = y0 + (y1 - y0) * j / n
            # Floor, mid-height and near-ceiling: a lamp overhead leaks onto
            # the floor, a bracket leaks at its own height.
            for z in (z0 + 4, (z0 + z1) / 2.0, z1 - 4):
                if name == "HALL" and 656 <= x <= 960 and -672 <= y <= -416:
                    continue  # inside the office's own footprint, not real hall floor
                pts.append((x, y, z))
    return pts


def _check_leaks():
    """Flag any `light_fixture` without `shadows=1` whose cone-and-range
    volume reaches into a room it isn't mounted in -- an unshadowed light in
    Bevy is occluded by nothing, so only its cone angle and `range` bound it
    (the Phase 21 light-leak bug: SPEC.md). Unlike `_check_overlaps` (which
    warns after writing), this is checked before any file is written and
    refuses to write at all -- a light leak isn't a "ship it and screenshot
    later" class of bug."""
    room_samples = {name: _leak_samples(name) for name in _LEAK_ROOMS}
    bad = []
    for lamp in LAMPS:
        if int(lamp.get("shadows", 0)):
            continue
        ox, oy, oz = (float(v) for v in lamp["origin"].split())
        home = _leak_room_of((ox, oy, oz))
        if home is None:
            continue
        cone = float(lamp.get("cone_deg", _DEFAULT_CONE_DEG))
        aim = _LEAK_AIM[lamp.get("aim", "down")]
        rng = float(lamp.get("range", _DEFAULT_RANGE_M)) * _UPM
        cos_half = math.cos(math.radians(cone / 2))
        for name, pts in room_samples.items():
            if name == home:
                continue
            hits = 0
            for px, py, pz in pts:
                dx, dy, dz = px - ox, py - oy, pz - oz
                dist = math.sqrt(dx * dx + dy * dy + dz * dz)
                if dist == 0 or dist > rng:
                    continue
                cosang = (dx * aim[0] + dy * aim[1] + dz * aim[2]) / dist
                if cosang >= cos_half:
                    hits += 1
            if hits:
                bad.append((lamp, home, name, hits))
    return bad


def main():
    bad_leaks = _check_leaks()
    if bad_leaks:
        print("LIGHT LEAK(S) FOUND -- refusing to write the map:\n")
        for lamp, home, into, hits in bad_leaks:
            print("  lamp %r at %s (in %s)" % (lamp.get("targetname"), lamp["origin"], home))
            print("    cone %s deg, range %s m, shadows OFF" % (
                lamp.get("cone_deg", _DEFAULT_CONE_DEG), lamp.get("range", _DEFAULT_RANGE_M)))
            print("    lit volume enters %s (%d sample pts)" % (into, hits))
            print("    fix: shadows=1, or trim cone/range, or move it\n")
        print("%d leak(s) found -- refusing to write the map." % len(bad_leaks))
        sys.exit(1)

    os.makedirs(MAP_DIR, exist_ok=True)
    for name, with_light in (("warehouse.map", False), ("warehouse_bsp_source.map", True)):
        path = os.path.join(MAP_DIR, name)
        with open(path, "w") as f:
            f.write(build(with_light))
        print("wrote", os.path.relpath(path))
    bad = _check_overlaps()
    if bad:
        print("\nWARNING: %d structural brush pair(s) share a volume (z-fight):" % len(bad))
        for amn, amx, at, bmn, bmx, bt, ov in bad:
            print("  %s%s [%s]  x  %s%s [%s]  overlap %s" % (amn, amx, at, bmn, bmx, bt, ov))


if __name__ == "__main__":
    main()
