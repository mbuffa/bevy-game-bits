# Compiles the TrenchBroom .map source into a lightmapped .bsp with
# ericw-tools (qbsp + light). See SPEC.md's Phase 5 section for tool
# provenance, why each flag is set, and why there are two .map sources
# instead of one (a live `light` entity in the dynamically-loaded test.map
# reintroduces the point-light white-out bug; test_bsp_source.map is a
# separate source that includes one, since baked BSP lighting sidesteps it).
#
# ericw-tools isn't packaged for Homebrew as of 2026-07; build it from
# source (SPEC.md has the submodule-init and -DDISABLE_DOCS=ON gotchas) and
# either put qbsp/light on PATH, or override on the command line:
#   make bsp QBSP=/path/to/qbsp LIGHT=/path/to/light

THIS_DIR   := $(dir $(abspath $(lastword $(MAKEFILE_LIST))))
REPO_ROOT  := $(abspath $(THIS_DIR)/../..)
ASSETS_DIR := $(REPO_ROOT)/assets
MAP_DIR    := $(ASSETS_DIR)/maps/immersive

QBSP  ?= qbsp
LIGHT ?= light

SOURCE := $(MAP_DIR)/test_bsp_source.map
BSP    := $(MAP_DIR)/test.bsp

# -qbism: Quake 2 BSP38 w/ expanded limits. Plain Q1 BSP strips brushes
#         unless compiled with -wrbrushesonly, and convex_collider() would
#         then find nothing to collide with -- the player falls forever.
# -path:  lets the compiler find loose textures under assets/textures/.
QBSP_FLAGS  := -qbism -nosubdivide -nosoftware -path $(ASSETS_DIR) -notex
LIGHT_FLAGS := -wrnormals -extra4 -lightgrid -path $(ASSETS_DIR) \
               -bounce 8 -bouncecolorscale 1 -bouncestyled 1 -dirt 1 -phong 1

# Compile byproducts alongside the .bsp that nothing at runtime reads.
BYPRODUCTS := $(MAP_DIR)/test.log $(MAP_DIR)/test-light.log $(MAP_DIR)/test.prt \
              $(MAP_DIR)/test.content.json $(MAP_DIR)/test.texinfo.json

.PHONY: bsp clean-bsp

bsp: $(BSP)

$(BSP): $(SOURCE)
	$(QBSP) $(QBSP_FLAGS) $< $@
	$(LIGHT) $(LIGHT_FLAGS) $@
	@rm -f $(BYPRODUCTS)

clean-bsp:
	rm -f $(BSP) $(BYPRODUCTS)
