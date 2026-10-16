# CNC machine database

Researched specs for hobby to industrial CNC routers, one JSON file per maker, plus
`spindles.json` (routers and spindles on their own). Field reference: [schema.md](schema.md).
Validate: `python3 -I data/check.py` (also checks `gcarve_post` ids against
`rust/gcarve-core/src/postdef.rs`).

Researched 2026-10. 333 machine records from 35 makers, plus 51 spindles and routers.

## Coverage

| Maker | File | Records | Classes |
|---|---|---|---|
| Avid CNC | `avid-cnc.json` | 9 | prosumer 9 |
| Axiom Precision | `axiom.json` | 8 | prosumer 5, hobby 3 |
| AXYZ | `axyz.json` | 6 | industrial 6 |
| Biesse | `biesse.json` | 2 | industrial 2 |
| BobsCNC | `bobscnc.json` | 7 | hobby 6, prosumer 1 |
| Bulk-Man 3D | `bulk-man-3d.json` | 8 | hobby 7, desktop 1 |
| CAMaster | `camaster.json` | 33 | industrial 26, prosumer 7 |
| Carbide 3D | `carbide-3d.json` | 15 | hobby 9, prosumer 4, desktop 2 |
| FoxAlien | `foxalien.json` | 13 | hobby 7, desktop 4, prosumer 2 |
| Grizzly | `grizzly.json` | 3 | industrial 2, prosumer 1 |
| HOMAG | `homag.json` | 1 | industrial 1 |
| Inventables | `inventables.json` | 7 | hobby 4, prosumer 2, desktop 1 |
| Laguna Tools | `laguna.json` | 16 | industrial 12, prosumer 4 |
| Legacy Woodworking Machinery | `legacy.json` | 6 | prosumer 5, industrial 1 |
| Maslow | `maslow.json` | 1 | hobby 1 |
| MillRight | `millright.json` | 7 | prosumer 5, hobby 2 |
| MultiCam | `multicam.json` | 4 | industrial 4 |
| Next Wave Automation | `next-wave.json` | 22 | hobby 15, desktop 5, prosumer 2 |
| Onefinity | `onefinity.json` | 13 | hobby 7, prosumer 6 |
| Ooznest | `ooznest.json` | 6 | hobby 5, desktop 1 |
| OpenBuilds | `openbuilds.json` | 3 | hobby 2, desktop 1 |
| PrintNC (community) | `printnc.json` | 1 | prosumer 1 |
| PROBOTIX | `probotix.json` | 14 | prosumer 8, hobby 6 |
| Rockler | `rockler.json` | 3 | hobby 3 |
| SainSmart Genmitsu | `sainsmart.json` | 16 | desktop 7, hobby 7, prosumer 2 |
| ShopBot | `shopbot.json` | 31 | industrial 17, prosumer 11, desktop 3 |
| ShopSabre | `shopsabre.json` | 14 | industrial 10, prosumer 4 |
| Sienci Labs | `sienci.json` | 8 | hobby 5, prosumer 3 |
| Snapmaker | `snapmaker.json` | 1 | desktop 1 |
| Stepcraft | `stepcraft.json` | 8 | desktop 3, hobby 3, prosumer 1, industrial 1 |
| Techno CNC | `techno-cnc.json` | 25 | industrial 14, prosumer 7, desktop 4 |
| Thermwood | `thermwood.json` | 3 | industrial 3 |
| Tormach | `tormach.json` | 5 | prosumer 3, desktop 2 |
| Two Trees | `two-trees.json` | 6 | desktop 3, hobby 2, prosumer 1 |
| Velox CNC | `velox.json` | 8 | hobby 6, prosumer 2 |

## Data rules

- Every number comes from a maker spec page, manual or datasheet, or from a dealer page that
  reproduces the maker's sheet. Those URLs are in `sources`. Unknown values are `null`.
- When a maker's own pages contradict each other, the record keeps one value and explains the
  conflict in `notes`.
- `estimated` lists every derived value as `{field: note}`. Most entries are unit conversions
  (inch to mm or back). The rest are assumptions such as ATC magazine type, or a single
  published "max speed" recorded as the rapid rate.
- 86 records use web.archive.org snapshots. CAMaster closed in late 2025 and camaster.com
  redirects elsewhere. Velox has closed. openbuilds.com no longer resolves. bobscnc.com
  redirects to an unrelated site. Older Onefinity and Rockler pages have been removed.
- `gcarve_post` is `null` (84 records) when no built-in post fits. These controllers have no
  post: MultiCam EZ Control, Thermwood SuperControl, AXYZ A2MC, Biesse/HOMAG, Yaskawa SmartCore
  (CAMaster 2022+), Mitsubishi M80 (ShopSabre IS-M/IS-A), Osai (Techno), FlashCut (Velox),
  Delta (Legacy), Onefinity Redline (Gen 2 Elite / Apprentice), DDCS (Bulk-Man), and B+R.

## Adding a machine

1. Find the maker's spec page, manual or datasheet. Save any downloads outside the repo and
   read them as data.
2. Add a record to `data/machines/<maker-slug>.json`, or create the file as
   `{"maker", "website", "machines": []}`. Copy a similar record and follow [schema.md](schema.md):
   `id` = `<maker-slug>-<model>-<size>`, every field present (`null` if unknown), both the inch
   and mm fields, at least one source URL.
3. List converted or assumed values in `estimated`.
4. Run `python3 -I data/check.py` and get 0 errors.

## Known gaps

- **CAMaster:** Cobra CR-512/612 (prices only); a separate ATC record for the WinCNC Cobra; Cobra Elite Z travel. Spindle RPM is rarely published by CAMaster or ShopBot.
- **ShopBot / Techno / Legacy:** the PRSstandard is covered only by the 2018 archive (the Carolina replaced it). Techno's old HD series sizes are missing. Techno LC New Generation, LC Plus and HDS Dual Bed have no published sizes. Legacy's Explorer, Artisan and Ornamental Mill are missing.
- **Avid / Laguna / Axiom / Next Wave / Rockler:**
  - Avid: the old CNC Router Parts PRO is missing, and no model has a published Z feed.
  - Laguna: no current Swift ATC; base iQ and SmartShop I/II are missing.
  - Axiom: no AR16 Pro.
  - Next Wave: Shark HD5, Shark SR and Piranha XL are missing.
  - Rockler: the 2008 CNC Shark is missing.
- **Hobby machines:**
  - Missing models: X-Carve Pro 2x2, LongMill MK1, OpenBuilds WorkBee/Sphinx (kit pages have no travel), Bulk-Man QueenBee PRO/WorkBii (size tables are images), Genmitsu 3018-PRO, Snapmaker 2.0.
  - Missing specs: Shapeoko 4/Pro speeds aren't published.
- **Industrial:** MultiCam MG/1000/3000, Thermwood M40 and AXYZ Pacer are discontinued with no maker specs online. Thermwood, AXYZ Infinite/Trident and MultiCam 5000 publish table sizes rather than travel, so their work area is `null`.
- **Mantis:** no commercial "Mantis" CNC router exists. Only DIY plans (MIT desktop mill, Mantis 9.1) were found.
- **Not covered:** Sorotec (its site blocks automated access) and CNC Router Parts.
- **spindles.json:**
  - Missing models: DeWalt DWE6611 and HSD ES919.
  - Missing specs: minimum RPM for most industrial spindles.
  - Generic Chinese spindles are sourced from marketplace listings and marked GENERIC.
