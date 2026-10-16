# Bit database schema

One file per maker: `data/bits/<maker>.json`

```json
{ "maker": "Whiteside", "source_summary": "...", "retrieved": "2026-10-08", "bits": [ <record>, ... ] }
```

Chip-load tables live in `data/bits/chipload.json` (see the end of this file).

## Bit record

Fields marked * are required. Optional fields are left out when unknown (never `null`, never guessed).

| field | type | meaning |
|---|---|---|
| `id`* | string | `<maker-slug>-<part>` lower-case, non-alphanumerics → `-` (e.g. `whiteside-ru2100`, `onsrud-52-280`). Unique across all files. |
| `maker`* | string | Display name ("Whiteside", "Amana Tool", "LMT Onsrud"...). |
| `part_number`* | string | Maker's part number exactly as published. Never invented. |
| `name`* | string | Short descriptive name (our wording, not catalog prose). |
| `family`* | enum | `end-mill-up` `end-mill-down` `compression` `mortise-compression` `straight` `v-bit` `ball-nose` `tapered-ball` `bowl` `surfacing` `drill` `chamfer` `engraving` `roundover` `ogee` `cove` `keyhole` `dovetail` `t-slot` `o-flute` `chipbreaker` `thread-mill` `other` |
| `tool_material` | string | `solid carbide`, `carbide tipped`, `HSS`, `PCD`, `diamond-coated`, `carbide insert`. |
| `coating` | string | e.g. `ZrN`, `AlTiN`, `Spektra (Amana nACo-type)`, `DLC`. |
| `units`* | `in`\|`mm` | Units of every length field in this record (as the maker publishes it). |
| `diameter`* | number | Cutting diameter. V-bits / engravers: largest cutting diameter. Tapered ball: the **tip** ball diameter goes in `tip_diameter`, `diameter` is the body diameter at the top of the flutes when published, else the tip diameter. |
| `shank` | number | Shank diameter. |
| `cut_length` | number | Flute length / cutting edge length. |
| `up_cut_length` | number | Compression bits: length of the up-cut portion at the tip. |
| `oal` | number | Overall length. |
| `flutes`* | integer ≥ 1 | Cutting edges (wings for surfacing/insert cutters). |
| `helix` | enum | `up` `down` `compression` `straight` `o-flute` |
| `angle_deg` | number | Included angle (V, engraving, chamfer, dovetail, carving liner). Tapered ball / tapered end mills: included taper angle (2 × per-side angle). |
| `tip_diameter` | number | Flat tip width of an engraving bit, or ball diameter of a tapered ball nose. |
| `tip_radius` | number | Ball radius (tapered ball, ball nose) or bowl-bit bottom radius. |
| `corner_radius` | number | Corner radius of bull-nose / bowl / core-box bits. |
| `max_rpm` | integer | Maker's maximum RPM if published. |
| `rpm` | [min, max] | Maker's recommended RPM range. |
| `chip_load` | {group: [min, max]} | Inches per tooth, only when the maker publishes it for this tool (or its exact series/size). Keys are material groups (below). |
| `chipload_ref` | string | Key into `chipload.json` `tables` when the chip load comes from a family/series table instead (e.g. `onsrud/52-200`, `amana/spiral-plunge-2f`). Must resolve. |
| `presets` | [object] | Maker's per-tool starting values: `{group, rpm, feed, plunge?, stepdown?, chip_load, source, note?}` (feed/plunge in/min, chip_load in/tooth). `group` is a material group or `any` when the maker gives one set for everything (e.g. Whiteside's Vectric/Fusion tool files). Used where the maker publishes a single value rather than a range. |
| `materials` | [string] | Materials the maker recommends the bit for (maker's words, shortened). |
| `applications` | [string] | e.g. `pocketing`, `profiling`, `v-carving`, `3d finishing`, `surfacing`. |
| `gcarve_tool`* | object | Direct mapping to GCarve's `Tool` (rust/gcarve-core/src/document.rs): `{name, kind: "EndMill"\|"VBit"\|"BallNose", diameter (inches), angle (VBit only), flutes}`. Tool has `#[serde(default)]`, so this object deserialises straight into a `Tool`; feeds come from the advisor. |
| `sources`* | [url] | Where every fact came from (catalog PDF, product page, maker chart). |
| `estimated` | bool | `true` if any value is derived rather than published (explained in `notes`). |
| `notes` | string | Caveats, conversions, maker data errors. |

GCarve kind mapping: V-bit / engraving / chamfer with an angle → `VBit`; ball nose and tapered ball → `BallNose` (tapered ball uses the tip ball diameter); everything else → `EndMill` (bowl, surfacing, straight, compression...).

## Material groups (chip-load keys)

| group | covers | GCarve `MaterialClass` (feeds.rs `classify_material`) |
|---|---|---|
| `softwood` | pine, fir, cedar, poplar, basswood | Softwood |
| `hardwood` | oak, maple, walnut, cherry | Hardwood |
| `plywood` | soft/hard plywood, Baltic birch, OSB | Plywood |
| `mdf` | MDF, HDF, particleboard | Mdf |
| `laminate` | melamine / laminated chipboard and plywood | Plywood (feeds.rs maps laminate/melamine there) |
| `plastic-soft` | HDPE, UHMW, PVC, PP, soft plastics | Plastic |
| `plastic-hard` | ABS, polycarbonate, acetal/Delrin, hard plastics | Plastic |
| `acrylic` | cast/extruded acrylic (PMMA) | Plastic |
| `solid-surface` | Corian and similar | Plastic |
| `aluminum` | aluminium sheet/plate, ACM | Aluminum |
| `brass` | brass, copper | Aluminum |
| `foam` | EPS/XPS, HDU sign foam, tooling board | Foam |
| `composite` | FRP, G10, carbon fibre, phenolic | (none; advisor should not guess) |
| `wood` | maker gives one value for "wood" / "natural wood" | Softwood and Hardwood |

All chip loads are **inches per tooth** (feed = RPM × flutes × chip load), for a depth of cut of 1 × D unless the table says otherwise. Onsrud and Amana both say: 2 × D → reduce 25 %, 3 × D → reduce 50 %.

## chipload.json

```json
{
  "units": "in/tooth",
  "groups": { "<group>": { "covers": "...", "gcarve_class": "Softwood" } },
  "tables": {
    "onsrud/52-200": {
      "maker": "LMT Onsrud", "series": ["52-200", "57-200"], "family": "end-mill-up",
      "description": "...", "depth_of_cut": "1xD", "rpm": 18000,
      "rows": { "softwood": { "0.25": [0.007, 0.009], ... }, ... },
      "sources": ["https://..."], "notes": "..."
    }
  }
}
```

`rows` keys are material groups; inner keys are the cutting diameter in inches as a decimal string (`"all"` = the maker gives one range for every diameter). A lookup takes the nearest published diameter (or interpolates), and never extrapolates far outside the published range.

Optional table fields: `flutes` (the chart is for this flute count), `rpm` (the chart's reference spindle speed), `single_values: true` (the maker prints one number per cell; stored as `[v, v]`), `omitted` (cells left out because the source is self-contradictory, e.g. min > max), `variant`, `family`, `description`.
