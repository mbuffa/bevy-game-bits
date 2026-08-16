# Contributing to 008-colony

Step-by-step recipes for extending the colony sim, written for humans and
coding agents alike. Each guide walks the full path of one kind of addition —
config constants, meshes/materials, components, spawn path, systems to
register, and the behaviors that can bite you — using an existing feature as
the worked example.

| I want to add a… | Read |
| --- | --- |
| (anything — start here for the lay of the land) | [00-architecture.md](00-architecture.md) |
| new resource / item kind (ore, food, …) | [01-add-an-item.md](01-add-an-item.md) |
| new building (wall/door-like constructible) | [02-add-a-building.md](02-add-a-building.md) |
| new job kind and/or work type (allowance + skill) | [03-add-a-job.md](03-add-a-job.md) |
| new zone kind or plantable crop/tree | [04-add-a-zone-or-plant.md](04-add-a-zone-or-plant.md) |
| new button, panel, tooltip line, or debug tab | [05-add-ui.md](05-add-ui.md) |
| new LDtk map element (terrain paint or placed entity) | [06-add-a-map-element.md](06-add-a-map-element.md) |
| new power producer, consumer, or storage kind | [07-add-power.md](07-add-power.md) |
| new pawn need (Sleep, Food, ...) | [08-add-a-need.md](08-add-a-need.md) |

## Conventions in these guides

- **Names, not line numbers.** Recipes reference types, functions, and consts
  (`director::generate_jobs`, `STACK_CAP`) plus their file. Line numbers rot;
  names are greppable.
- **The code wins.** These guides describe the code as it is. If a guide and
  the code disagree, the code is right — fix the guide in the same change.
- **Lean on the compiler.** Most extension points are exhaustive `match`es on
  an enum. Add the variant first, then `cargo check --example 008-colony` and
  let the compile errors walk you to every arm that needs a decision. Each
  guide calls out the places the compiler *cannot* find (UI lists, generation
  logic, LDtk field declarations) — those are what the checklists are for.
- **Verify by running.** `cargo run --example 008-colony`, backtick opens the
  debug window (Jobs tab shows the live job pool), Space pauses. Unit tests:
  `cargo test --example 008-colony`. Screenshots taken while verifying belong
  in `screenshots/008-colony/` with a date prefix (see the repo `CLAUDE.md`).
