# CNC router bit database

Facts only: part numbers, dimensions, flute counts, geometry, and the maker's recommended RPM and chip loads.
Every record carries the URL it came from. Schema: [schema.md](schema.md). Validate with `python3 -I data/check.py`
(`check_bits()`).

- `<maker>.json`: one file per maker, records in the shape of `schema.md`. Each record has a `gcarve_tool` object that
  deserialises straight into `gcarve_core::document::Tool` (kind `EndMill` / `VBit` / `BallNose`, diameter in inches,
  angle, flutes), so it can go into the tool library as it is.
- `chipload.json`: maker chip-load tables, indexed by bit family or series, cutting diameter and material group
  (inches per tooth). The feeds advisor (`rust/gcarve-core/src/feeds.rs`) should use these. Material groups map onto
  `MaterialClass` through `groups.<g>.gcarve_class`.

Retrieved 2026-10-08. The catalogs and charts were downloaded to a scratch folder and are not in the repo.

## Coverage

| maker | file | bits | end-mill-up | end-mill-down | compression | mortise-compression | o-flute | chipbreaker | straight | ball-nose | tapered-ball | v-bit | engraving | surfacing | bowl | other families* | metric | chip_load | chipload_ref | presets | estimated |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| Amana Tool | amana.json | 1097 | 361 | 184 | 96 | 43 | 42 | 20 | 15 | 84 | 51 | 64 | 23 | 39 | 6 | 69 | 115 | 0 | 620 | 11 | 0 |
| Bits & Bits Co. | bitsbits.json | 156 | 38 | 10 | 8 | 0 | 5 | 0 | 0 | 73 | 6 | 13 | 0 | 0 | 2 | 1 | 28 | 0 | 0 | 0 | 8 |
| Carbide 3D | carbide3d.json | 30 | 12 | 1 | 1 | 0 | 3 | 0 | 0 | 5 | 0 | 6 | 2 | 0 | 0 | 0 | 6 | 0 | 28 | 0 | 0 |
| CMT | cmt.json | 107 | 15 | 16 | 17 | 10 | 0 | 2 | 2 | 6 | 4 | 19 | 4 | 8 | 4 | 0 | 13 | 0 | 0 | 0 | 0 |
| DATRON | datron.json | 113 | 105 | 2 | 0 | 0 | 0 | 0 | 0 | 6 | 0 | 0 | 0 | 0 | 0 | 0 | 113 | 85 | 0 | 0 | 0 |
| Freud | freud.json | 309 | 133 | 127 | 20 | 19 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 10 | 0 | 0 | 251 | 0 | 0 | 0 | 0 |
| Harvey Tool | harvey.json | 89 | 52 | 23 | 0 | 0 | 0 | 0 | 0 | 14 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 89 | 0 | 0 |
| IDC Woodcraft | idc-woodcraft.json | 75 | 13 | 14 | 7 | 0 | 3 | 0 | 0 | 8 | 6 | 15 | 0 | 4 | 2 | 3 | 0 | 0 | 0 | 72 | 1 |
| Inventables (incl. Cyclone Bits sold by Inventables) | inventables.json | 38 | 12 | 7 | 2 | 0 | 2 | 0 | 5 | 4 | 5 | 1 | 0 | 0 | 0 | 0 | 2 | 0 | 0 | 0 | 0 |
| LMT Onsrud | onsrud.json | 926 | 163 | 123 | 78 | 0 | 315 | 37 | 118 | 39 | 14 | 12 | 20 | 6 | 0 | 1 | 168 | 0 | 834 | 0 | 0 |
| Sienci Labs | sienci.json | 13 | 1 | 2 | 2 | 0 | 0 | 0 | 0 | 2 | 2 | 3 | 0 | 1 | 0 | 0 | 13 | 10 | 0 | 0 | 0 |
| SpeTool | spetool.json | 306 | 75 | 35 | 35 | 1 | 28 | 0 | 5 | 16 | 28 | 26 | 5 | 38 | 4 | 10 | 110 | 0 | 208 | 285 | 2 |
| Vortex Tool | vortex.json | 554 | 105 | 104 | 69 | 45 | 135 | 36 | 16 | 19 | 0 | 0 | 6 | 19 | 0 | 0 | 90 | 0 | 554 | 0 | 6 |
| Whiteside | whiteside.json | 737 | 63 | 53 | 33 | 9 | 28 | 6 | 165 | 26 | 2 | 13 | 7 | 6 | 8 | 318 | 209 | 0 | 401 | 168 | 349 |
| Yonico | yonico.json | 120 | 22 | 24 | 12 | 0 | 16 | 4 | 0 | 9 | 4 | 28 | 0 | 0 | 0 | 1 | 0 | 0 | 0 | 0 | 5 |
| **total** | | **4670** | 1170 | 725 | 380 | 127 | 577 | 105 | 326 | 311 | 122 | 200 | 67 | 131 | 26 | 403 | 1118 | 95 | 2734 | 536 | 371 |

\* other families: drill 189 (Whiteside dowel drills and hinge boring bits), roundover 64, dovetail 56, other 35 (Amana tapered flat-bottom carving bits, Whiteside cabinet-door profile bits, SpeTool juice-groove bits etc.), chamfer 17, ogee 13, cove 10, t-slot 9, keyhole 7. `chip_load` = per-tool range published by the maker; `presets` = per-tool single starting values; `chipload_ref` = family/series table in chipload.json.

Coverage targets:
- **Whiteside and Amana CNC spirals.** Up, down and compression, 1/8"–1/2", on 1/4" and 1/2" shanks: all
  covered. Whiteside RU/RD/UD, plus Amana 46xxx with their Spektra (-K) and ZrN (-Z) versions.
- **V-bits.**
  - Whiteside has 60°, 90° and 120°, plus 15/22/30/45° engravers and liners.
  - Amana has 60°, 90°, 120° and 150°, plus 30/60/90° engravers and the ACM folding bits.
- **Ball nose and tapered ball:** Whiteside, Amana, Onsrud (52-200B/BL, 65-200B/300B, 77-100), CMT, SpeTool, IDC Woodcraft.
- **Surfacing:**
  - Whiteside 6210/6220/SB.
  - Amana 455xx carbide tipped and RC-22xx insert cutters, with per-tool feeds.
  - Onsrud 91-000.
  - Vortex, Freud NC12M, SpeTool, IDC Woodcraft.
- **Onsrud core series:** 52-200, 52-200B/BL, 52-400 (metric), 52-6/7/900, 57-000/200/400/600/900,
  60-000…60-950 including every 60-100 compression line, 61/62/63/64/65-xxx O-flutes, 77-100 taper.
- **Metric sizes:** Onsrud, Amana, Whiteside, Freud (mostly metric), Vortex, SpeTool, Datron (all metric), Sienci,
  Carbide 3D "El Ocho".

## Sources by maker

| maker | where the records come from |
|---|---|
| Whiteside | whitesiderouterbits.com product pages (Shopify product data: the "TOOL SPECS" line, materials, recommended and max RPM). Flute counts come from the section headings of the 2027 full catalog PDF (e.g. "STANDARD SPIRAL BITS … Two Flute", O-flute table "Flutes" column) or Whiteside's Fusion 360 tool library. Starting feeds come from Whiteside's Vectric `.tool` files (Dropbox link on their site) and are stored as `presets`. Dowel drills and hinge boring bits: size from each product page's subtitle ("9mmCD, 70mmOAL") and size table. |
| Amana Tool | Amana's product tables on toolstoday.com, Amana's own retail site; amanatool.com refuses scripted access, but its `pub/media/productattachments` PDFs download fine. Speed charts: spiral plunge 2/3F (Spektra), compression 1/2/3F, spiral ball nose, ball nose for plastics, 15/60/90° V-groove, plastic and aluminium O-flute, 3F chipbreaker finisher, spoilboard 2-wing and 2+2. |
| LMT Onsrud | 2025 Production Cutting Tools catalog (`onsrud.com/images/2025_PCT_Catalog.pdf`, product offering tables, with the page number in each source URL) + the 13 "Cutting Data Recommendations" PDFs (one per material) on onsrud.com. |
| Vortex Tool | Vortex catalog PDF + Vortex's single chip-load chart (image-only PDF; values read from the rendered page). |
| Freud | Freud general catalogue, CNC section (SCH solid carbide, NC12M surfacing). |
| CMT | CMT USA 2026 catalog PDF. |
| SpeTool | spetools.com store product data (spetool.com is a parked domain), plus SpeTool's own downloads (spetools.com/pages/spetool-tool-file-database): the Fusion 360 tool library (geometry for parts missing from the store, and per-material presets), the Carbide Create CSV database (presets where the Fusion library has none) and the feed & speed chart PDFs (`spetool/*` tables: spiral up/down/compression, O-flute, tapered ball, engraving, V-bit, carbide-tipped V-bit, bowl, core box). |
| Harvey Tool | harveytool.com series tables + Harvey's per-series speeds & feeds PDFs (aluminium and plastics). |
| DATRON | shop.datron.com product pages + DATRON High-Speed Cutting Guide (per-tool fz). |
| Carbide 3D | shop.carbide3d.com product spec tables. |
| IDC Woodcraft | Shopify product data + IDC's Carbide Create tool library CSV (per-tool presets, softwood, desktop machine) + IDC feeds & speeds PDF (`idc-woodcraft/*` tables, single values). IDC's download page puts a personal-use licence on its files; only facts were taken, but check before shipping. |
| Inventables | Shopify product data. Most of the newer line is Cyclone Bits (kept in inventables.json with maker "Cyclone Bits", id prefix `cyclone-`); several listings cross-reference Onsrud part numbers. |
| Bits & Bits | bitsbits.com product pages (RPM where listed; no chart). |
| Sienci Labs | sienci.com product pages + LongMill feeds & speeds PDF (`sienci/*` tables: range = "reduced" to "full" level). |
| Yonico | precisionbits.com product pages (page 1 of each CNC category; listing pagination is behind Cloudflare). |

## How chip loads were derived

1. **A maker chart, by series or family** (`chipload.json` → `tables`, records point at it with `chipload_ref`).
   - **Onsrud.** Parsed from the 13 per-material PDFs by column position. Each Onsrud series gets one table
     (`onsrud/52-200`, `onsrud/60-100MW` …), with the published `[min, max]` per diameter and material.
     - Material mapping: Soft/Hard Wood → `softwood`/`hardwood`; Soft/Hard Plywood → `plywood`/`plywood-hard`;
       MDF → `mdf`; Laminated Chipboard and Laminated Plywood → `laminate`, using the wider span where the two differ;
       Soft/Hard Plastic → `plastic-soft`/`plastic-hard`; Solid Surface; Aluminum; Foam; Composite.
     - One printed cell is self-contradictory: 40-000 hard wood 3/8", ".009-.007". It is left out and listed under `omitted`.
   - **Amana.** Transcribed from the speed-chart PDFs.
     - Most Amana charts print one number per cell, not a range. Those tables are marked `single_values: true`
       and stored as `[v, v]`.
     - Amana's "Wood" column goes into the `wood` group (feeds.rs Softwood + Hardwood). "MDF/Laminate" goes into
       both `mdf` and `laminate`.
   - **Vortex, Harvey, Sienci, IDC Woodcraft.** Merged from the per-maker builders (`vortex/general`, `harvey/sf-*`,
     `sienci/*`, `idc-woodcraft/*`).
   - **Harvey (all series), Helical, Kyocera SGS, Garr, Kodiak, 2L, LakeShore** (round 2, 2026-10-09). Harvey and Helical
     from the per-series speeds & feeds PDFs (Harvey: [slotting, finishing] for aluminium/brass, [slot, profile] for plastics,
     wood and composite rows; Helical: [slot, roughing], wrought aluminium and copper-alloy rows). SGS from the Global Catalog
     v26.1 chart pages, Garr from its General Purpose Milling Guide, Kodiak from its catalog and micro end mill chart, 2L from
     its end mill table, LakeShore from its VFA sheet (image PDF). Kodiak prints diameter ranges (keyed by the upper size).
     Micro cells below 0.0002 in/tooth are left out.
   - **SpeTool.** Transcribed from SpeTool's chart PDFs (one value per cell, `single_values: true`). The tapered-ball
     chart is keyed by tip ball diameter (0.5–4 mm, 1/32–1/8 in). SpeTool says 2 × D: reduce feed 30 %, 3 × D: 50 %.
     The surfacing chart gives feeds only (no chip load), so it is not a table; surfacing bits get the library presets.
2. **The maker's per-tool numbers.**
   - When the maker gives a range for this exact tool (Datron's per-part fz), it goes in `chip_load`.
   - When it gives a single starting value, it goes in `presets` with rpm and feed, and chip load = feed / (rpm ×
     flutes). Examples: Whiteside's Vectric/Fusion files (one preset for "any" material), Amana's spoilboard charts
     (one value per material).
3. **No chart from the maker.** Whiteside, Carbide 3D and some others publish none, so `chipload_ref` points at the
   closest family table from another maker, and the record's `notes` says so:
   - Whiteside up/down spirals → Onsrud 52-200 / 57-200.
   - Compression → Onsrud 60-100MW.
   - Ball nose → Amana spiral ball nose.
   - V-bits → Amana V-groove.
   - Tapered ball → Onsrud 77-100.

   Carbide-tipped profile bits (bowl, roundover, dovetail, keyhole…) get no reference. No maker publishes chip
   loads for them, and the feeds.rs default applies.

**Depth of cut.** All charts are for 1 × D. Onsrud and Amana both say to reduce the chip load 25 % at 2 × D and
50 % at 3 × D.

**Converting to GCarve's classes.** Use `groups.<g>.gcarve_class`:
- `plywood-hard` and `laminate` → Plywood.
- `plastic-*`, `acrylic` and `solid-surface` → Plastic.
- `brass` → Aluminum.
- `wood` → both Softwood and Hardwood.
- `composite` → no class.

## Where makers disagree

The tables follow each maker exactly, and the gaps between them are large:

- **Compression, 1/4", wood.**
  - Amana compression 2F: 0.0031 in/tooth (MDF 0.0061).
  - Onsrud 60-100MW: softwood 0.018–0.020 (MDF 0.013–0.015).
  - That is about 6× apart for the same geometry. Vortex's chart is higher still for 1/2" in MDF: 0.025–0.027,
    against Amana's 0.0111.
- **Up-cut spiral, 1/4".**
  - Onsrud 52-200: 0.007–0.009 softwood, 0.006–0.008 MDF.
  - Amana spiral plunge: 0.005 wood, 0.006 MDF.
  - Whiteside's Vectric starting values: 0.004 (160 IPM at 20,000 RPM, 2F), meant for light machines.
- **Amana's 1/2" spiral plunge is lower than its 3/8"** (0.0057 vs 0.0064 in wood). That is as printed.
- **Single O-flute in plastics.**
  - Amana: 0.008–0.012 at 1/4".
  - Onsrud 63-700 hard plastic: 0.010–0.012.
  - feeds.rs uses 0.004–0.006 for "plastic" at 1/4", which is only right for 2-flute tools.
- **feeds.rs compared with the makers.**
  - MDF 1/4": feeds.rs uses 0.011–0.013. Onsrud's 2F spiral says 0.006–0.008; its compressions say 0.013–0.015.
  - Softwood 1/4": feeds.rs 0.009–0.011 is above Onsrud 52-200 (0.007–0.009).
  - Aluminium (0.003–0.004 at 1/4") agrees with Amana and Onsrud O-flute (0.003–0.006).
- **Hobby charts against industrial charts.**
  - IDC's benchtop chip loads are about 3–4× below Onsrud/Amana. At 1/4" down-cut: IDC 0.0021, Onsrud 57-200
    0.007–0.009.
  - Sienci's LongMill chart sits in between, at 0.0027–0.0045.
  - The advisor should pick the table that matches the machine class.
- **Harvey and Datron aluminium numbers are machining-centre numbers.** Datron is around 0.003–0.008 in/tooth at
  30–40k RPM.
- **Tapered ball angles are per side.** Amana, CMT, SpeTool and Onsrud ("angle per side") publish the per-side
  angle, e.g. 5.4° for a 1/16" × 1/4" bit. Whiteside's SC64/SC66 "included angle" values (11°, 7°) match the same
  geometry. `angle_deg` is always the included angle.

## Maker data errors found

- **Onsrud 60-186MW.** Up-cut length printed as "4" (longer than its 2-1/4" LOC). Omitted.
- **Onsrud 40-000.** The hard wood 3/8" cell is printed as ".009-.007". Omitted.
- **Amana chart typo.** The ball-nose chart's 1/8" sign-foam feed (110–190 IPM) does not match its chip load
  (0.005–0.007). The chip load is kept.
- **Amana metric spiral-plunge chart.** Prints chip load in metres (0.00013 = 0.13 mm).
- **Amana listings where the title and spec table disagree.** The table value is used and the record notes the title
  value (e.g. 45702, 45806). 46143 (title 5/64", table 1/64") is left out.
- **Whiteside's Fusion 360 library** describes RD5100 (a down-cut) as "Upcut".
- **Datron** lists R = 3 mm on a 4 mm ball nose. Corrected to D/2 and noted.
- **Sienci's 1/4" compression bit** lists its flute type as "Downcut".
- **IDC TBN-1:** the product page gives 0.015" as the tip diameter, the IDC tool library gives it as the radius.
  Marked `estimated`.
- **Freud** has one catalog page with the shank and overall-length columns swapped (SCH3DFN 14–20 mm). Corrected and
  noted.

## Gaps

- **Whiteside.**
  - Hercules Max Life up/down (HU/HD): the catalog section does not say how many flutes they have.
  - SC90/SC91 fiberglass bits, 1540, the solid-carbide dovetails D8-250/D75-25, and the larger round-nose
    (core box) bits 1407–1420: no flute count is published.
  - Carbide-tipped form bits (V-groove, bowl, dovetail, keyhole, T-slot, straight, roundover, cove, ogee, edge
    bevel, cabinet-door profiles), RFT/RFTD flush-trim spirals and the dowel drills / hinge boring bits: flutes set to 2
    and marked `estimated` where neither page nor catalog says.
  - Bearing-guided profile bits (roundover, cove, ogee, edge bevel, round nose, bowl & tray) and the flush-trim
    spirals are included (usable on a CNC with an offset toolpath). Left-hand spirals/straights, template, stile-and-rail
    sets, slotting cutters, saw blades, hole saws and countersinks are excluded. Left-hand dowel drills and hinge boring
    bits are included (CNC drill blocks run both rotations).
  - Edge-bevel bits (2298–2325): Whiteside's bevel angle is kept in `notes` only (it is not a V included angle).
  - 1409 and 3720 are in Whiteside's Vectric files but no longer on the store: not added.
  - Product page typo: 1203/1205 print the cutting diameter as '1-2"'; 1/2" from the catalog.
- **Amana.**
  - 45760–45777 signmaking engravers: no flute column.
  - TE- tenon cutters: not included.
  - Profile bits with bearings, sets and 5-packs: excluded.
  - Some tables cover only the Spektra (-K) part numbers. Uncoated twins point at the same chart.
- **Onsrud.**
  - Honeycomb, aircraft, PCD/DFC composite, drills, trim blades, dor-bits, 40-50, 47-00 and 66-000 (plastic edge
    rounding): not included.
  - 63-400 is not included because its table layout differs.
  - Some series have no cutting-data row: 60-100EC, 62-100, 63-100, 54-775, 11-00. Those records have no `chipload_ref`.
- **No chip-load data published:** Freud (graphs only), CMT, Carbide 3D (machine-specific feed charts
  only), IDC Woodcraft.
- **SpeTool.** Steel/iron presets (M-series) are not stored: no material group for steel. Flush-trim, bearing-template
  and Domino tenon cutters are left out; 4 EU/UK listings carry only a SKU code (e.g. SP2FC-D6-L22), no model number,
  and are left out. The aluminium end-mill chart PDF did not OCR cleanly; the Fusion library's aluminium presets cover
  those bits instead.
- **Yonico:** only the first listing page per category (120 bits).
- **Bits & Bits:** 105 house-brand products have no published flute count or diameter.
- **Inventables:** most V-bits have no cutting diameter or flute count.
- **Not covered:** Kyocera SGS, and Whiteside "Spektra" (Spektra is Amana's coating, already covered under Amana).
  The ToolsToday house brand has no router bits of its own; its products are Amana or Vectric bundles.
- **Missing materials.** No maker chart covers brass for router bits, except Harvey's aluminium/brass charts.
  Acrylic has its own rows only in Amana's O-flute plastic chart, Harvey's charts and Vortex's chart.
