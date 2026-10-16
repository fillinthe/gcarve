# Machine database schema

One file per manufacturer: `data/machines/<maker-slug>.json`. Validate with `python3 -I data/check.py`.

```json
{"maker": "CAMaster", "website": "https://www.camaster.com", "machines": [ ... ]}
```

Unknown values are `null` (never guessed). Numbers come from a maker spec page, manual or
datasheet listed in `sources`. A value that is derived or estimated is listed in `estimated`
with a note (see below).

## Machine record

| field | type | notes |
|---|---|---|
| `id` | string, kebab-case, unique across all files | `camaster-panther-5x8` |
| `maker` | string | same as the file's `maker` |
| `model` | string | `Panther` |
| `variant` | string or null | size / trim: `5x8`, `ATC`, `Pro` |
| `years` | string or null | `2015-`, `2017-2021` |
| `class` | `desktop` \| `hobby` \| `prosumer` \| `industrial` | |
| `work_area_in` | `{x, y, z}` numbers or null each | cutting travel, inches |
| `work_area_mm` | `{x, y, z}` | same in mm (must agree with inches within 2 %) |
| `gantry_clearance_in` | number or null | clearance under the gantry |
| `spindles` | list of spindle options (below), may be empty | |
| `max_feed_ipm` / `max_feed_mm_min` | `{xy, z}` or null | max cutting feed |
| `rapid_ipm` / `rapid_mm_min` | `{xy, z}` or null | rapid traverse |
| `accel` | string or null | as published, with units (`0.5 g`, `1000 mm/s²`) |
| `drive` | `{xy, z}` each `rack-pinion` \| `ballscrew` \| `belt` \| `leadscrew` \| null | `xy` may be `"belt/leadscrew"` style text when X and Y differ |
| `motors` | `stepper` \| `closed-loop` \| `servo` \| null | `closed-loop` = closed-loop/hybrid steppers |
| `controllers` | list of `{name, gcarve_post, default}` | `gcarve_post` = id from `rust/gcarve-core/src/postdef.rs` or null |
| `tool_change` | `{kind, slots, notes}` | kind: `manual` \| `atc-linear` \| `atc-carousel` \| `atc-drum` \| `multi-head` \| `quick-change`; slots int or null |
| `tool_setter` | `{available, type}` | bool; type e.g. `fixed tool length sensor`, `touch plate`, `BitSetter` |
| `z_zero` | list of strings | `top of material`, `table (spoilboard)`, `touch plate`, `tool setter` |
| `table` | `{hold_down: [..], max_material_thickness_in}` | hold_down values: `vacuum`, `T-track`, `spoilboard`, `clamps`, `threaded inserts`, `slats`... |
| `sources` | list of URLs, at least one | maker pages first |
| `estimated` | object, optional | `{"<field path>": "note"}`, e.g. `{"work_area_mm": "converted from inches"}` |
| `notes` | string or null | |

## Spindle option

| field | type | notes |
|---|---|---|
| `type` | `router` \| `spindle` | trim routers (Makita, DeWalt) = `router` |
| `name` | string | `Makita RT0701C`, `2.2 kW HSD ES915` |
| `power_kw`, `power_hp` | number or null | |
| `rpm_min`, `rpm_max` | int or null | routers: dial range |
| `collet` | string or null | `ER20`, `1/4" + 1/2"`, `ISO30` |
| `cooling` | `air` \| `water` \| null | |
| `vfd` | bool or null | |
| `default` | bool | the one shipped in the base package |

## spindles.json

`{"spindles": [ ... ]}`, each: `id`, `maker`, `model`, `type`, `power_kw`, `power_hp`, `rpm_min`,
`rpm_max`, `collet`, `collet_min_in`, `collet_max_in` (shank sizes the stock/optional collets cover),
`cooling`, `vfd`, `voltage`, `sources`, `notes`, optional `estimated`.
