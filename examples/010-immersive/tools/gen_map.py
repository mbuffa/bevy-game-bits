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
Warehouse interior 1280 x 896 x 512 u (32.5 x 22.8 x 13.0 m). Two decks at
z 176..192 (4.88 m); a 512 u (13.0 m) gap between them -- past any jump. Ladder
on deck A's south face, running 16 u above the deck so the top rungs are
grabbable. Deck B has no ladder -- the player reaches it by climbing
`prop_crate` props (carry.rs, Phase 8): a big fixed 1.6 m crate is pre-placed
against its south edge, and 3 loose 0.8 m crates sit nearby to stack on top.
Two more crates on the spawn line drive the autopilot.

Phase 11 (+ round 3/4) adds a straight run EAST of the warehouse's east wall,
all of it at x > 656 (nothing west of that moves): a door-sized hole in the
east wall -> a corridor (x 656..CORRIDOR_END, ~3.15 m tall) -> a tall brick
`bay` room (x CORRIDOR_END..ROOM_END, y ±336, z 0..300) -> an untextured
open-square yard (x ROOM_END..+800, y -556..556, z 0..360; a `skip` lid seals
it for `make bsp`, skybox later). The bay's far wall has a large HANGAR_*
opening to the yard where a roll-up hangar door will eventually go -- left
un-doored for now.

Phase 13 props: a locked `prop_door` (breakable.rs / door.rs) fills the
east-wall doorway -- the only door -- and a wooden crate holding a "lockpick"
plus a "crowbar" pickup sit on platform B (reached only by the crate stack).
Throw the crate off the deck to shatter it and drop the lockpick.

Lighting (Phase 9 + 12): 6 ceiling + 2 aisle `light_fixture` lamps on the
"main_lights" circuit, now *on* at load; 4 always-on "night" bracket lamps, one
on each deck support pillar; 2 "corridor" wall brackets + 3 "bay" ceiling lamps
+ 3 "yard" floods down the east extension, all always on; a `func_light_switch`
on deck B's north wall (z 256, hand height) -- with its own PointLight beacon --
that still toggles the whole "main_lights" bank (it just starts on now).
"""

import os

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


def box_brush(mins, maxs, tex, tex_top=None, tex_bottom=None):
    """A solid axis-aligned box. `tex` is the default; `tex_top`/`tex_bottom`
    override the +Z / -Z faces."""
    x0, y0, z0 = mins
    x1, y1, z1 = maxs
    assert x0 < x1 and y0 < y1 and z0 < z1, (mins, maxs)
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

# Interior: x -640..640, y -448..448, z 0..512. 16 u shell.
#
# The east wall (x 640..656) carries a door-sized hole at y -DOORWAY_HW..HW,
# z 0..DOORWAY_H — the mouth of the Phase 11 corridor. Everything WEST of x 656
# is byte-for-byte what Phase 9 left (the 009-world-map "grow, don't move"
# rule: every crate / lamp / switch / ladder origin and both autopilot scripts
# are pinned to it).
DOORWAY_HW = 22    # door-opening half-width (u); DOOR_WIDTH is ~1.12 m ≈ 44 u
DOORWAY_H = 90     # door-opening height (u) ≈ 2.3 m
CORRIDOR_HW = 30   # corridor interior half-width (u) ≈ 1.5 m
CORRIDOR_H = 124   # corridor interior height (u) ≈ 3.15 m
CORRIDOR_END = 1120  # corridor east limit (u); the bay's west wall butts here
ROOM_END = 1800    # bay east outer face (u) == the yard's west outer face
HANGAR_HW = 84     # bay->yard opening half-width (u) ≈ 4.3 m — a future hangar door
HANGAR_H = 128     # bay->yard opening height (u) ≈ 3.25 m

WORLDSPAWN = [
    # floor / ceiling -- both concrete
    box_brush((-656, -464, -16), (656, 464, 0), FLOOR, tex_top=FLOOR),
    box_brush((-656, -464, 512), (656, 464, 528), FLOOR),
    # walls: west, south, north
    box_brush((-656, -464, -16), (-640, 464, 528), WALL),
    box_brush((-640, -464, -16), (640, -448, 528), WALL),
    box_brush((-640, 448, -16), (640, 464, 528), WALL),
    # east wall, split around the corridor doorway (y ±DOORWAY_HW, z 0..DOORWAY_H)
    box_brush((640, -464, -16), (656, -DOORWAY_HW, 528), WALL),
    box_brush((640, DOORWAY_HW, -16), (656, 464, 528), WALL),
    box_brush((640, -DOORWAY_HW, DOORWAY_H), (656, DOORWAY_HW, 528), WALL),
    # platform A (top-left, ladder deck) and platform B (top-right)
    box_brush((-640, 64, 176), (-256, 448, 192), DECK, tex_top=DECK),
    box_brush((256, 64, 176), (640, 448, 192), DECK, tex_top=DECK),
    # four support columns under the decks -- iron, same as the decks they hold
    box_brush((-352, 112, 0), (-320, 144, 176), DECK),
    box_brush((-576, 112, 0), (-544, 144, 176), DECK),
    box_brush((320, 112, 0), (352, 144, 176), DECK),
    box_brush((544, 112, 0), (576, 144, 176), DECK),
]

# --- east extension: corridor -> bay -> exterior yard ------------------------
#
# A straight run east from the warehouse's east wall. Purely additive — all of
# it is at x > 656. The `prop_door` (Phase 13) sits in the DOORWAY_* hole in
# the east wall above; the corridor beyond it is a touch wider than the doorway
# so the leaf has somewhere to swing.
#
# Round 3 deleted the intermediate room, round 4 put it back as a **bay** — a
# tall staging room whose far wall has a large opening to the exterior where a
# *hangar door* will eventually go (`FuncDoor`, `angle -1` = roll up). For now
# that opening is left un-doored: you just walk through.
#
# **Abut, don't overlap.** Each segment stops where the next begins — two
# brushes sharing a face is fine, two sharing a *volume* z-fights (round 2).
#
#   corridor  x 656..CORRIDOR_END          y ±CORRIDOR_HW  z 0..CORRIDOR_H
#   bay       x CORRIDOR_END..ROOM_END      y ±336          z 0..300
#   yard      x ROOM_END..+800   y -556..556  z 0..360      (open square, `skip` lid)

CORRIDOR = [
    box_brush((656, -CORRIDOR_HW, -16), (CORRIDOR_END, CORRIDOR_HW, 0), FLOOR, tex_top=FLOOR),
    box_brush((656, -CORRIDOR_HW, CORRIDOR_H), (CORRIDOR_END, CORRIDOR_HW, CORRIDOR_H + 16), FLOOR),
    box_brush((656, -CORRIDOR_HW - 16, -16), (CORRIDOR_END, -CORRIDOR_HW, CORRIDOR_H + 16), WALL),
    box_brush((656, CORRIDOR_HW, -16), (CORRIDOR_END, CORRIDOR_HW + 16, CORRIDOR_H + 16), WALL),
]

# Bay: a tall brick room. West wall split around the corridor mouth (corridor
# cross-section, no door); east wall split around the large HANGAR_* opening to
# the yard (no door yet). Ceiling at z 300 leaves lintel room for a future
# roll-up door.
_BAY_HW = 336
BAY = [
    box_brush((CORRIDOR_END, -_BAY_HW, -16), (ROOM_END, _BAY_HW, 0), FLOOR, tex_top=FLOOR),
    box_brush((CORRIDOR_END, -_BAY_HW, 300), (ROOM_END, _BAY_HW, 316), FLOOR),
    # west wall around the corridor mouth
    box_brush((CORRIDOR_END, -_BAY_HW, -16), (CORRIDOR_END + 16, -CORRIDOR_HW, 316), WALL),
    box_brush((CORRIDOR_END, CORRIDOR_HW, -16), (CORRIDOR_END + 16, _BAY_HW, 316), WALL),
    box_brush((CORRIDOR_END, -CORRIDOR_HW, CORRIDOR_H), (CORRIDOR_END + 16, CORRIDOR_HW, 316), WALL),
    # south / north walls
    box_brush((CORRIDOR_END, -_BAY_HW, -16), (ROOM_END, -_BAY_HW + 16, 316), WALL),
    box_brush((CORRIDOR_END, _BAY_HW - 16, -16), (ROOM_END, _BAY_HW, 316), WALL),
    # east wall around the hangar opening (left OPEN for now)
    box_brush((ROOM_END - 16, -_BAY_HW, -16), (ROOM_END, -HANGAR_HW, 316), WALL),
    box_brush((ROOM_END - 16, HANGAR_HW, -16), (ROOM_END, _BAY_HW, 316), WALL),
    box_brush((ROOM_END - 16, -HANGAR_HW, HANGAR_H), (ROOM_END, HANGAR_HW, 316), WALL),
]

# Yard: an untextured open square (skybox later). The `skip` lid renders
# nothing at runtime but seals the volume so `make bsp` (ericw-tools qbsp)
# doesn't leak on the open sky. Its west wall is split around the same
# HANGAR_* opening the bay's east wall has, abutting at ROOM_END.
_YARD_W = ROOM_END              # yard west outer face
_YARD_E = ROOM_END + 800        # yard east outer face
YARD = [
    box_brush((_YARD_W, -556, -16), (_YARD_E, 556, 0), FLOOR, tex_top=FLOOR),
    box_brush((_YARD_W, -556, 360), (_YARD_E, 556, 376), "skip"),
    box_brush((_YARD_E - 16, -556, -16), (_YARD_E, 556, 376), WALL),
    box_brush((_YARD_W, -556, -16), (_YARD_E, -540, 376), WALL),
    box_brush((_YARD_W, 540, -16), (_YARD_E, 556, 376), WALL),
    # west wall around the hangar opening (matches the bay's east opening)
    box_brush((_YARD_W, -556, -16), (_YARD_W + 16, -HANGAR_HW, 376), WALL),
    box_brush((_YARD_W, HANGAR_HW, -16), (_YARD_W + 16, 556, 376), WALL),
    box_brush((_YARD_W, -HANGAR_HW, HANGAR_H), (_YARD_W + 16, HANGAR_HW, 376), WALL),
]

WORLDSPAWN += CORRIDOR + BAY + YARD

# Ladder brush: bolted to deck A's south face (y 64), 48 u wide, from the
# floor to 16 u above the deck. Its own func_ladder entity so ladder.rs can
# turn it into a Sensor and read its AABB. `skip`-textured -> invisible; the
# rails-and-rungs mesh is built in ladder.rs. Deliberately generous (40 u
# deep) as a grab volume, not a ladder's real footprint -- the *solid*
# collider that stops the player is on that mesh (LADDER_VISUAL_DEPTH thick,
# ~4 u), sized in ladder.rs, not on this brush.
LADDER_BRUSH = box_brush((-472, 24, 0), (-424, 64, 208), LADDER_TEX)

# --- crates ---------------------------------------------------------------
#
# Carryable metal props (`prop_crate`, see classes.rs / carry.rs). Three groups:
#
#  * AUTOPILOT_CRATES -- on the line *behind* the spawn (away from the ladder),
#    so `IMMERSIVE_AUTOPILOT=crates` reaches them by turning 180 deg and walking
#    forward, and the `IMMERSIVE_AUTOPILOT=1` ladder walk (which heads the other
#    way) never touches them. One normal, one over-`CARRY_MAX_MASS`.
#  * BIG_CRATE -- a 1.6 m ~800 kg crate against platform B's south edge. Too
#    heavy to lift (so it's a fixed step) and, at ~1/3 the deck height, an
#    affordance that says "climb here".
#  * STACKING_CRATES -- 3 loose 0.8 m crates beside it. Hop onto the big crate,
#    stack 2 of these on top (1.6 + 0.8 + 0.8 = 3.2 m), jump onto platform B
#    (deck top 4.88 m, jump 1.8 m -> 5.0 m). The 3rd is a spare.
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

# Between platform B's support columns (x 320-352, 544-576); north face ~y 61,
# ~3 u off the deck's south edge (y 64).
BIG_CRATE = _crate(448, 30, mass=800, size=1.6, prompt="Heavy crate")
STACKING_CRATES = [
    _crate(360, -20),   # west of the base
    _crate(448, -90),   # south
    _crate(536, -20),   # east of the base
]

# --- props (Phase 10) --------------------------------------------------------
#
# The hinged door (`prop_door`, door.rs), the breakable wooden crate
# (`prop_wood_crate`, breakable.rs) and two `item_pickup` models (items.rs).
# Placed loose on the open floor a few metres east of spawn (y -256, x -448),
# clear of both the ladder walk and the autopilot crate line — Phase 13 moves
# them to where they belong (crate + crowbar onto platform B, door into the new
# corridor). `prop_door` origin is the HINGE at floor level; `prop_wood_crate`
# and `item_pickup` origins are the model centre, so z = size/2 rests them on
# the floor (top z 0).


def _door(x, y, face_yaw, locked=0, prompt="Open door"):
    # `x` sits the ~0.08 m leaf just inside the near wall face (not the wall
    # mid-line) so you don't see daylight around a thin leaf in a thick brush.
    # `y` is the HINGE — on a jamb (`DOORWAY_HW`). `width`/`height` are DERIVED
    # from the doorway hole so the leaf and the opening can't drift apart;
    # `door.rs` then laps the leaf `DOOR_REBATE` over the frame on top of that.
    return {"classname": "prop_door",
            "origin": "%d %d 0" % (x, y),
            "face_yaw": str(face_yaw), "locked": str(locked), "prompt": prompt,
            "width": "%.4f" % (2 * DOORWAY_HW / _UPM),
            "height": "%.4f" % (DOORWAY_H / _UPM)}


def _wood_crate(x, y, contains, size=0.6, z_base=0,
                prompt="Wooden crate — throw it to break it open"):
    # origin is the cube centre; z_base is the surface it rests on (0 = floor,
    # 192 = platform B deck top).
    return {"classname": "prop_wood_crate",
            "origin": "%d %d %d" % (x, y, z_base + round(size * _UPM / 2)),
            "size": str(size), "contains": contains, "prompt": prompt}


def _pickup(key, x, y, z):
    return {"classname": "item_pickup", "origin": "%d %d %d" % (x, y, z), "item": key}


# Phase 13 — the props in their real places, once Phase 10 verified the models
# and the door swing on the spawn line:
#
#  * the locked `prop_door` fills the corridor doorway in the warehouse's east
#    wall (the DOORWAY_* hole). Hinge on the north jamb (TB y = DOORWAY_HW),
#    `face_yaw 0` so the closed leaf lies across the opening and swings east
#    into the corridor, away from a player approaching from the warehouse.
#    It's the ONLY door — pick it (Phase 16) and walk the corridor to the yard.
#  * the wooden crate (holding the lockpick) and the crowbar sit on platform B
#    (deck top z 192), reached only by the Phase 8 crate stack. Throw the crate
#    off the deck onto the concrete to break it open — or crowbar it in place
#    (Phase 16).
#
# No loose "lockpick" pickup any more — it only exists once the crate breaks.
_DECK_B_TOP = 192
PROPS = [
    _door(643, DOORWAY_HW, face_yaw=0, locked=1, prompt="Open door"),
    _wood_crate(392, 168, contains="lockpick", z_base=_DECK_B_TOP),
    _pickup("crowbar", 452, 168, _DECK_B_TOP + 18),
]

# --- lighting (Phase 9) --------------------------------------------------
#
# The warehouse loads dark: `GlobalAmbientLight` at `config::AMBIENT_DARK`, no
# sun, the overhead `light_fixture` bank *off*. A single `func_light_switch`
# on platform B's north wall (reached by the crate stack — B has no ladder)
# turns the "main_lights" circuit on. Four bracket lamps — one on each platform
# support pillar — are on `targetname "night"`, which no switch drives, so they
# stay lit: the always-on aid for the dark spawn + the crate-stacking corner,
# and the proof the group filter works.
#
# `light_fixture` is a point class (classes.rs): origin is the lamp's mount
# point (a ceiling/deck-underside/pillar surface — `lights.rs` embeds the
# connector 0.02 m into it), `targetname` its circuit, `aim` ("down" default,
# or a compass word for a raked wall/pillar bracket), and every SpotLight knob
# (intensity, range, cone_deg, color, shadows) defaults from `config::LAMP_*`.


def _lamp(x, y, z, targetname, **over):
    d = {"classname": "light_fixture", "origin": "%d %d %d" % (x, y, z),
         "targetname": targetname}
    for k, v in over.items():
        d[k] = str(v)
    return d


# Ceiling bank: 3 x 2 grid seated on the 512 u ceiling. The y +224 row lights
# the two decks, the y -224 row the open south floor. One lamp — over the
# crate-stacking spot at platform B's foot — casts shadows, so a growing stack
# reads as a growing shadow; the rest don't (keep the shadow-map count tiny).
#
# Phase 12: `start_on=1` — the warehouse now loads LIT. The platform-B switch
# still toggles the whole "main_lights" circuit; it just starts on. (The dark
# room is one flip away, and `IMMERSIVE_LIGHTS=toggle` now proves lit→dark→lit.)
CEILING_LAMPS = [
    _lamp(x, y, 512, "main_lights", start_on=1,
          **({"shadows": 1} if (x, y) == (448, -224) else {}))
    for x in (-448, 0, 448)
    for y in (-224, 224)
]

# Aisle lamps seated on the deck underside (z 176) so the space beneath the
# platforms isn't a black hole.
AISLE_LAMPS = [
    _lamp(-448, 256, 176, "main_lights", start_on=1, cone_deg=110),
    _lamp(448, 256, 176, "main_lights", start_on=1, cone_deg=110),
]

# Pillar night lights: a bracket on the south face (y 112) of each of the four
# deck support pillars (TB x -352..-320 / -576..-544 for platform A, 320-352 /
# 544-576 for B), ~3 u up, raked south (`aim`). Platform A's pair (brighter,
# wider) throw a glow toward the spawn 9 m south; B's pair light the
# crate-stacking corner. `targetname "night"` — no switch drives it, so they
# stay lit through the toggle.
PILLAR_NIGHT_LAMPS = [
    _lamp(-336, 112, 118, "night", aim="south", intensity=300000,
          color="255 210 160", cone_deg=130, start_on=1),   # platform A
    _lamp(-560, 112, 118, "night", aim="south", intensity=300000,
          color="255 210 160", cone_deg=130, start_on=1),
    _lamp(336, 112, 118, "night", aim="south", intensity=250000,
          color="255 210 160", cone_deg=120, start_on=1),    # platform B
    _lamp(560, 112, 118, "night", aim="south", intensity=250000,
          color="255 210 160", cone_deg=120, start_on=1),
]

# East-extension lamps. Their own `targetname` circuits, none of which the
# platform-B switch drives, all `start_on=1` — the corridor and yard are places
# you pass through, always lit.
#
# The corridor is only ~1.5 m wide and ~3 m tall, so a hanging `"down"` lamp
# puts its shade at eye level and a bracket that aims *across* the corridor
# reaches the centre line. These aim *along* the corridor (`aim "east"` /
# `"west"`, TB +x/-x): `lights::spawn_fixtures` runs the arm parallel to the
# wall and rakes the SpotLight down the length, so the shade hugs the wall near
# the ceiling. Two of them, opposite walls, one washing each half.
EXTENSION_LAMPS = [
    _lamp(736, -CORRIDOR_HW, CORRIDOR_H - 10, "corridor", aim="east",
          start_on=1, intensity=260000, cone_deg=120, color="255 235 200"),
    _lamp(CORRIDOR_END - 80, CORRIDOR_HW, CORRIDOR_H - 10, "corridor", aim="west",
          start_on=1, intensity=260000, cone_deg=120, color="255 235 200"),
    # Bay: ceiling `"down"` lamps — the room is ~7.6 m tall, plenty of headroom.
    _lamp(1320, -160, 300, "bay", start_on=1, cone_deg=110),
    _lamp(1320, 160, 300, "bay", start_on=1, cone_deg=110),
    _lamp(1600, 0, 300, "bay", start_on=1, cone_deg=110),
    _lamp(_YARD_W + 260, -260, 340, "yard", start_on=1, color="220 230 255"),
    _lamp(_YARD_W + 260, 260, 340, "yard", start_on=1, color="220 230 255"),
    _lamp(_YARD_W + 620, 0, 340, "yard", start_on=1, color="220 230 255"),
]

LAMPS = CEILING_LAMPS + AISLE_LAMPS + PILLAR_NIGHT_LAMPS + EXTENSION_LAMPS

# The wall switch. A 40 x 12 x 48 u `skip` box standing 12 u proud of platform
# B's north wall (inner face y 448), centred at z 256 — 1.6 m above the deck
# top (z 192), hand height. `skip` is invisible but still yields a collider
# (the aim target); lights.rs draws a fixed-size lit panel + red PointLight
# from the AABB (the func_ladder precedent).
SWITCH_BRUSH = box_brush((396, 436, 232), (436, 448, 280), "skip")
# Phase 12: `start_on=1` to match the now-lit ceiling bank. `lights.rs` picks
# the initial prompt from this ("Turn off the lights").
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
// A tall warehouse (1280 x 896 x 512 u) with two raised platforms 512 u
// apart. Platform A (west) carries a ladder to the floor; platform B (east)
// has no ladder -- climb the pre-placed big crate at its south edge and stack
// two loose crates on top. B holds a wooden crate (lockpick inside) and a
// crowbar; throw the crate off the deck to break it. The room loads lit; a
// wall switch on B still toggles the ceiling bank. A locked door in the east
// wall opens onto a corridor -> bay room -> open yard (a large hangar-door
// opening in the bay's far wall, un-doored for now).
// See SPEC.md Phases 7-16.
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


def main():
    os.makedirs(MAP_DIR, exist_ok=True)
    for name, with_light in (("warehouse.map", False), ("warehouse_bsp_source.map", True)):
        path = os.path.join(MAP_DIR, name)
        with open(path, "w") as f:
            f.write(build(with_light))
        print("wrote", os.path.relpath(path))


if __name__ == "__main__":
    main()
