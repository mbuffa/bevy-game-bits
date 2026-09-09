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

    assets/maps/immersive/warehouse.map             -- no `light` entity; the
        example lights the room with a DirectionalLight in main.rs (a live
        point light triggers the Phase 1 white-out on the dynamic-load path).
    assets/maps/immersive/warehouse_bsp_source.map  -- adds `light` and
        `info_player_start` for the ericw-tools compile (`make bsp`).

Run from anywhere:  python3 examples/010-immersive/tools/gen_map.py
Deterministic: re-running produces byte-identical files.

Geometry (TrenchBroom units, +Z up, +Y north; 39.37008 u/m)
----------------------------------------------------------
Interior 1280 x 896 x 512 u (32.5 x 22.8 x 13.0 m). Two decks at z 176..192
(4.88 m); a 512 u (13.0 m) gap between them -- past any jump. Ladder on deck
A's south face, running 16 u above the deck so the top rungs are grabbable.
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

FLOOR = "dev_gray_floor"
WALL = "dev_gray_wall"
DECK = "dev_gray_deck"
# `skip` is in bevy_trenchbroom's default `auto_remove_textures`: the brush
# renders nothing but still contributes a convex collider. ladder.rs builds the
# visible rails-and-rungs mesh itself from this brush's AABB.
LADDER_TEX = "skip"

# Interior: x -640..640, y -448..448, z 0..512. 16 u shell.
WORLDSPAWN = [
    # floor / ceiling
    box_brush((-656, -464, -16), (656, 464, 0), FLOOR, tex_top=FLOOR),
    box_brush((-656, -464, 512), (656, 464, 528), WALL),
    # walls: west, east, south, north
    box_brush((-656, -464, -16), (-640, 464, 528), WALL),
    box_brush((640, -464, -16), (656, 464, 528), WALL),
    box_brush((-640, -464, -16), (640, -448, 528), WALL),
    box_brush((-640, 448, -16), (640, 464, 528), WALL),
    # platform A (top-left, ladder deck) and platform B (top-right)
    box_brush((-640, 64, 176), (-256, 448, 192), DECK, tex_top=DECK),
    box_brush((256, 64, 176), (640, 448, 192), DECK, tex_top=DECK),
    # four support columns under the decks
    box_brush((-352, 112, 0), (-320, 144, 176), WALL),
    box_brush((-576, 112, 0), (-544, 144, 176), WALL),
    box_brush((320, 112, 0), (352, 144, 176), WALL),
    box_brush((544, 112, 0), (576, 144, 176), WALL),
]

# Ladder brush: bolted to deck A's south face (y 64), 48 u wide, from the
# floor to 16 u above the deck. Its own func_ladder entity so ladder.rs can
# turn it into a Sensor and read its AABB. `skip`-textured -> invisible; the
# rails-and-rungs mesh is built in ladder.rs. Deliberately generous (40 u
# deep) as a grab volume, not a ladder's real footprint -- the *solid*
# collider that stops the player is on that mesh (LADDER_VISUAL_DEPTH thick,
# ~4 u), sized in ladder.rs, not on this brush.
LADDER_BRUSH = box_brush((-472, 24, 0), (-424, 64, 208), LADDER_TEX)

PLAYER_SPAWN = {"classname": "player_spawn", "origin": "-448 -256 48", "angle": "90"}
# NB: no "angle" key — bevy_trenchbroom would read it as a brush rotation and
# throw the ladder across the room. `face_yaw` is plain data ladder.rs reads.
LADDER_ENTITY = {"classname": "func_ladder", "face_yaw": "90", "prompt": "Climb ladder"}
LIGHT = {"classname": "light", "origin": "0 0 440", "light": "600"}
# qbsp's leak-fill occupant; classname is literal. Our player_spawn coexists.
INFO_PLAYER_START = {"classname": "info_player_start", "origin": "-448 -256 48", "angle": "90"}

HEADER = """\
// 010-immersive warehouse map -- GENERATED by
// examples/010-immersive/tools/gen_map.py, do not edit by hand.
//
// A tall warehouse (1280 x 896 x 512 u) with two raised platforms 512 u
// apart. Platform A (west) carries a ladder to the floor; platform B (east)
// is reachable only by climbing A and... there is no bridge yet -- the gap
// is the point. See SPEC.md Phase 7.
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
    parts.append(_entity(PLAYER_SPAWN))
    if with_light:
        parts.append(_entity(INFO_PLAYER_START))
        parts.append(_entity(LIGHT))
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
