// Golden generator for the Rust CAM port: runs the C++ engine on fixed inputs and writes every case (inputs + all
// output moves) as JSON. Build + run: sh rust/testdata/cam/gen/build.sh (writes rust/testdata/cam/*.json).
#include <cstdio>
#include <fstream>
#include <functional>
#include <sstream>
#include <string>
#include <vector>

#include "cam.h"
#include "document.h"
#include "import.h"
#include "relief.h"
#include "sim.h"
#include "wincnc.h"

using namespace gcarve;

// ---- tiny JSON writer -----------------------------------------------------------------------------------------------
static std::string num(double v) { char b[40]; std::snprintf(b, sizeof b, "%.17g", v); return b; }
static std::string str(const std::string& s) {
    std::string o = "\"";
    for (char c : s) {
        if (c == '"' || c == '\\') { o += '\\'; o += c; }
        else if (c == '\n') o += "\\n";
        else if (c == '\r') o += "\\r";
        else if ((unsigned char)c < 0x20) { char b[8]; std::snprintf(b, sizeof b, "\\u%04x", c); o += b; }
        else o += c;
    }
    return o + "\"";
}
static std::string opt(const std::optional<double>& v) { return v ? num(*v) : "null"; }
template <class T, class F> static std::string arr(const std::vector<T>& v, F f) {
    std::string o = "[";
    for (size_t i = 0; i < v.size(); ++i) { if (i) o += ","; o += f(v[i]); }
    return o + "]";
}

static const char* toolType(ToolType t) { return t == ToolType::VBit ? "VBit" : t == ToolType::BallNose ? "BallNose" : "EndMill"; }
static const char* sideName(Side s) { return s == Side::Inside ? "Inside" : s == Side::On ? "On" : "Outside"; }
static const char* entryName(Entry e) { return e == Entry::Plunge ? "Plunge" : e == Entry::Ramp ? "Ramp" : "Spiral"; }
static const char* opType(OpType t) {
    static const char* n[] = {"Profile", "Pocket", "Drill", "VCarve", "Bore", "Inlay", "Flute", "Relief", "Chamfer", "Prism"};
    return n[int(t)];
}

static std::string jTool(const Tool& t) {
    return "{\"name\":" + str(t.name) + ",\"kind\":\"" + toolType(t.type) + "\",\"diameter\":" + num(t.diameter) + ",\"angle\":" + num(t.angle) +
           ",\"stepdown\":" + num(t.stepdown) + ",\"feed_xy\":" + num(t.feedXY) + ",\"feed_z\":" + num(t.feedZ) + ",\"rpm\":" + std::to_string(t.rpm) +
           ",\"flutes\":" + std::to_string(t.flutes) + "}";
}
static std::string jMat(const Material& m) {
    return "{\"kind\":" + str(m.type) + ",\"w\":" + num(m.w) + ",\"h\":" + num(m.h) + ",\"t\":" + num(m.t) + ",\"safe_z\":" + num(m.safeZ) +
           ",\"retract_z\":" + num(m.retractZ) + ",\"origin\":" + std::to_string(m.origin) + ",\"z_bottom\":" + (m.zBottom ? "true" : "false") + "}";
}
static std::string jOp(const Op& o) {
    std::ostringstream s;
    s << "{\"kind\":\"" << opType(o.type) << "\",\"name\":" << str(o.name) << ",\"tool\":" << o.tool << ",\"side\":\"" << sideName(o.side)
      << "\",\"depth\":" << num(o.depth) << ",\"stepdown\":" << num(o.stepdown) << ",\"entry\":\"" << entryName(o.entry) << "\",\"climb\":" << (o.climb ? "true" : "false")
      << ",\"tabs\":" << (o.tabs ? "true" : "false") << ",\"tab_count\":" << o.tabCount << ",\"tab_width\":" << num(o.tabWidth) << ",\"tab_height\":" << num(o.tabHeight)
      << ",\"tab_ramped\":" << (o.tabRamped ? "true" : "false") << ",\"ramp_length\":" << num(o.rampLength) << ",\"stepover\":" << num(o.stepover)
      << ",\"allowance\":" << num(o.allowance) << ",\"peck\":" << num(o.peck) << ",\"flat_depth\":" << num(o.flatDepth) << ",\"start_depth\":" << num(o.startDepth)
      << ",\"glue_gap\":" << num(o.glueGap) << ",\"inlay_margin\":" << num(o.inlayMargin) << ",\"ramp_in\":" << num(o.rampIn) << ",\"ramp_out\":" << num(o.rampOut) << ",\"chamfer_width\":" << num(o.chamferWidth)
      << ",\"clear_tool\":" << o.clearTool << ",\"raster_angle\":" << o.rasterAngle << ",\"rough\":" << (o.rough ? "true" : "false")
      << ",\"rough_stepdown\":" << num(o.roughStepdown) << ",\"rough_stepover\":" << num(o.roughStepover) << "}";
    return s.str();
}
static std::string jPt(Pt p) { return "[" + num(p.x) + "," + num(p.y) + "]"; }
static std::string jContour(const Contour& c) {
    return std::string("{\"closed\":") + (c.closed ? "true" : "false") + ",\"nodes\":" + arr(c.nodes, [](const Node& n) {
        return "[" + num(n.p.x) + "," + num(n.p.y) + "," + num(n.in.x) + "," + num(n.in.y) + "," + num(n.out.x) + "," + num(n.out.y) + "," + (n.hasIn ? "1" : "0") + "," + (n.hasOut ? "1" : "0") + "]";
    }) + "}";
}
static std::string jMove(const Move& m) {
    return "[" + std::to_string(int(m.type)) + "," + opt(m.x) + "," + opt(m.y) + "," + opt(m.z) + "," + num(m.i) + "," + num(m.j) + "]";
}
static std::string jPath(const Toolpath& t) {
    return "{\"name\":" + str(t.name) + ",\"tool\":" + std::to_string(t.tool) + ",\"feed_xy\":" + num(t.feedXY) + ",\"feed_z\":" + num(t.feedZ) + ",\"moves\":" + arr(t.moves, jMove) + "}";
}
static std::string jStrs(const std::vector<std::string>& v) { return arr(v, str); }

// ---- inputs ---------------------------------------------------------------------------------------------------------
static Contour star() {   // the 4_VCARVE star of samples/compare/compare.dxf
    return makePolyline({{8, 3.35}, {7.8237, 2.8427}, {7.2867, 2.8318}, {7.7147, 2.5073}, {7.5592, 1.9932}, {8, 2.3}, {8.4408, 1.9932}, {8.2853, 2.5073}, {8.7133, 2.8318}, {8.1763, 2.8427}}, true);
}
// text-like concave shapes: an "E" (polyline), an "O" (ellipse with an elliptic counter), a curvy "S"-ish cubic blob
static std::vector<Contour> letters() {
    std::vector<Contour> v;
    v.push_back(makePolyline({{1, 1}, {1.8, 1}, {1.8, 1.15}, {1.15, 1.15}, {1.15, 1.45}, {1.6, 1.45}, {1.6, 1.6}, {1.15, 1.6}, {1.15, 1.85}, {1.8, 1.85}, {1.8, 2}, {1, 2}}, true));
    v.push_back(makeEllipse(2.5, 1.5, 0.45, 0.5));
    v.push_back(makeEllipse(2.5, 1.5, 0.25, 0.32));
    Builder b; b.start({3.2, 1}); b.cubic({3.9, 0.9}, {4.0, 1.5}, {3.5, 1.5}); b.cubic({3.1, 1.5}, {3.2, 2.1}, {3.9, 2.0}); b.line({3.9, 1.85});
    b.cubic({3.4, 1.9}, {3.35, 1.62}, {3.55, 1.62}); b.cubic({4.15, 1.62}, {4.05, 0.8}, {3.2, 0.85}); b.close();
    v.push_back(b.c);
    return v;
}

struct Case { std::string name; Op op; Tool tool; const Tool* clear = nullptr; Material m; std::vector<Contour> cs; };

int main(int argc, char** argv) {
    std::string root = argc > 1 ? argv[1] : ".";   // repo root
    std::string outDir = argc > 2 ? argv[2] : root + "/rust/testdata/cam/";
    Material mat{"Plywood", 48, 48, 0.484};
    std::vector<Tool> tools = defaultTools();
    Tool em = tools[0], vb = tools[2];
    vb.diameter = 1.0;   // as in cam_test: shapes fit the bit
    Pt c{1.675, 1.675};
    auto circ = [](double x, double y, double r) { return makeEllipse(x, y, r, r); };

    std::vector<Case> cases;
    auto add = [&](std::string name, Op op, Tool t, std::vector<Contour> cs, const Tool* clear = nullptr, Material m = Material{"Plywood", 48, 48, 0.484}) {
        cases.push_back({std::move(name), std::move(op), std::move(t), clear, m, std::move(cs)});
    };
    // profile
    Op in; in.side = Side::Inside; in.depth = 0.484; in.climb = false;
    Op out; out.depth = 0.484; out.climb = false;
    add("profile_inside_circle_conventional", in, em, {circ(c.x, c.y, 0.458333)});
    add("profile_outside_circle_conventional", out, em, {circ(c.x, c.y, 0.925)});
    { Op o = out; o.climb = true; add("profile_outside_circle_climb", o, em, {circ(c.x, c.y, 0.925)}); o.side = Side::Inside; add("profile_inside_circle_climb", o, em, {circ(c.x, c.y, 0.458333)}); }
    add("profile_disc_with_hole", out, em, {circ(c.x, c.y, 0.925), circ(c.x, c.y, 0.458333)});
    { Op t = out; t.tabs = true; t.tabCount = 4; t.tabWidth = 0.25; t.tabHeight = 0.125; t.entry = Entry::Plunge; add("profile_rect_square_tabs_plunge", t, em, {makeRect(0, 0, 10, 5)}); }
    { Op t = out; t.depth = 0.5; t.entry = Entry::Plunge; t.tabs = true; t.tabCount = 2; t.tabWidth = 0.3; t.tabHeight = 0.2; t.tabRamped = true; add("profile_rect_3d_tabs", t, em, {makeRect(0, 0, 10, 1)}); }
    { Op t = out; t.depth = 0.5; t.tabs = true; t.tabCount = 3; t.tabRamped = true; t.climb = true; add("profile_circle_3d_tabs_spiral", t, em, {circ(3, 3, 1.5)}); }
    { Op r = out; r.depth = 0.4; r.stepdown = 0.4; r.entry = Entry::Ramp; r.rampLength = 1.0; add("profile_rect_ramp", r, em, {makeRect(0, 0, 4, 2)}); }
    { Op r = out; r.depth = 0.45; r.stepdown = 0.2; r.entry = Entry::Ramp; r.rampLength = 0.7; r.tabs = true; r.tabCount = 2; add("profile_rect_ramp_multipass_tabs", r, em, {makeRect(0, 0, 4, 2)}); }
    { Op o = out; o.side = Side::On; o.depth = 0.3; o.stepdown = 0.1; o.entry = Entry::Plunge; add("profile_open_on_plunge", o, em, {makePolyline({{0, 0}, {2, 0.5}, {3, 2}}, false)}); o.entry = Entry::Ramp; o.rampLength = 0.5; add("profile_open_on_ramp", o, em, {makePolyline({{0, 0}, {2, 0.5}, {3, 2}}, false)}); }
    { Op o = out; o.depth = 0.2; add("profile_open_warns_outside", o, em, {makePolyline({{0, 0}, {2, 0.5}}, false)}); }
    { Op o = out; o.climb = true; o.depth = 0.75; o.stepdown = 0.25; o.tabs = true; add("profile_star_climb_tabs", o, em, {star()}, nullptr, Material{"Plywood", 12, 8, 0.75}); }
    { Op o = out; o.side = Side::On; o.depth = 0.1; add("profile_star_on", o, em, {star()}); }
    { Op o = out; o.climb = true; o.depth = 0.25; add("profile_letters_outside", o, tools[1], letters()); o.side = Side::Inside; add("profile_letters_inside", o, tools[1], letters()); }
    add("profile_tiny_inside", in, em, {circ(0, 0, 0.1)});
    { Op o = out; o.depth = 0; add("profile_zero_depth", o, em, {circ(1, 1, 1)}); }
    { Op o = out; o.depth = 1.0; add("profile_deeper_than_board", o, em, {circ(c.x, c.y, 0.925)}); }
    { Op o = out; o.depth = 0.5; o.stepdown = 0.25; add("profile_three_squares_order", o, em, {makeRect(10, 10, 11, 11), makeRect(0.5, 0.5, 1.5, 1.5), makeRect(10, 0.5, 11, 1.5)}); }
    // pocket
    Op pk; pk.type = OpType::Pocket; pk.name = "Pocket"; pk.depth = 0.25; pk.entry = Entry::Plunge;
    add("pocket_rect", pk, em, {makeRect(0, 0, 2, 1)});
    add("pocket_island", pk, em, {makeRect(0, 0, 3, 3), makeRect(1, 1, 2, 2)});
    add("pocket_two_areas", pk, em, {makeRect(0, 0, 1, 1), makeRect(3, 0, 4, 1)});
    { Op p = pk; p.allowance = 0.1; add("pocket_allowance", p, em, {makeRect(0, 0, 2, 1)}); }
    add("pocket_too_small", pk, em, {makeRect(0, 0, 0.2, 0.2)});
    { Op p = pk; p.entry = Entry::Ramp; p.rampLength = 0.5; add("pocket_ramp", p, em, {makeRect(0, 0, 2, 1)}); p.depth = 0.5; p.rampLength = 20; add("pocket_ramp_long", p, em, {makeRect(0, 0, 2, 1)}); }
    { Op p = pk; p.climb = false; p.depth = 0.6; p.stepdown = 0.25; add("pocket_circle_conventional_multipass", p, em, {circ(8, 5, 1)}); }
    { Op p = pk; p.stepover = 0.4; add("pocket_compare_layer", p, em, {makeRect(2, 4, 5, 6), circ(8, 5, 1)}, nullptr, Material{"Plywood", 12, 8, 0.75}); }
    { Op p = pk; p.stepover = 0.3; add("pocket_star_eighth", p, tools[1], {star()}); }
    { Op p = pk; add("pocket_letters_eighth", p, tools[1], letters()); }
    { Op p = pk; add("pocket_open_skipped", p, em, {makePolyline({{0, 0}, {2, 2}}, false), makeRect(0, 0, 2, 1)}); }
    // drill
    Op dr; dr.type = OpType::Drill; dr.name = "Drill"; dr.depth = 0.4; dr.peck = 0.15;
    add("drill_peck", dr, em, {circ(1, 1, 0.125), circ(3, 1, 0.125)});
    add("drill_big_hole", dr, em, {circ(1, 1, 0.5)});
    { Op d = dr; d.peck = 0; add("drill_straight", d, em, {circ(1, 1, 0.1), circ(5, 1, 0.1), circ(2, 4, 0.1), circ(1.2, 1.1, 0.1)}); }
    // v-carve
    Op vc; vc.type = OpType::VCarve; vc.name = "VCarve"; vc.tool = 2;
    add("vcarve_square", vc, vb, {makeRect(0, 0, 0.8, 0.8)});
    { Tool narrow = vb; narrow.diameter = 0.5; add("vcarve_square_narrow_bit", vc, narrow, {makeRect(0, 0, 0.8, 0.8)}); }
    add("vcarve_ellipse", vc, vb, {makeEllipse(1, 1, 0.4, 0.25)});
    { Op v = vc; v.flatDepth = 0.2; add("vcarve_flat_depth", v, vb, {makeRect(0, 0, 0.8, 0.8)}); }
    add("vcarve_too_wide", vc, vb, {makeRect(0, 0, 4, 4)});
    add("vcarve_counter", vc, vb, {makeRect(0, 0, 1, 1), makeRect(0.4, 0.4, 0.6, 0.6)});
    { Op v = vc; v.flatDepth = 0.1; v.clearTool = 0; add("vcarve_clearance_tool", v, vb, {makeRect(0, 0, 2, 2)}, &tools[0]); }
    { Op v = vc; v.flatDepth = 0.1; v.clearTool = 0; Tool big = em; big.diameter = 3; static Tool sbig; sbig = big; add("vcarve_clearance_too_big", v, vb, {makeRect(0, 0, 2, 2)}, &sbig); }
    { Op v = vc; v.startDepth = 0.1; v.flatDepth = 0.3; v.clearTool = 0; add("vcarve_start_depth", v, vb, {makeRect(0, 0, 0.8, 0.8)}, &tools[0]); add("vcarve_start_depth_no_endmill", v, vb, {makeRect(0, 0, 0.8, 0.8)}); }
    { Op v = vc; v.startDepth = 0.3; v.flatDepth = 0.2; add("vcarve_start_below_flat", v, vb, {makeRect(0, 0, 0.8, 0.8)}); }
    add("vcarve_star_compare", vc, tools[2], {star()}, nullptr, Material{"Plywood", 12, 8, 0.75});
    add("vcarve_letters", vc, tools[2], letters());
    { Op v = vc; v.stepdown = 0.1; add("vcarve_layered_passes", v, vb, {makeRect(0, 0, 0.9, 0.7)}); }
    { Op v = vc; v.flatDepth = 0.15; v.stepdown = 0.05; add("vcarve_letters_flat_layers", v, tools[2], letters()); }
    { Op v = vc; v.flatDepth = 0.05; v.clearTool = 1; add("vcarve_letters_flat_clear", v, tools[2], letters(), &tools[1]); }
    add("vcarve_endmill_refused", vc, em, {makeRect(0, 0, 0.8, 0.8)});
    add("vcarve_open_skipped", vc, vb, {makePolyline({{0, 0}, {1, 1}}, false)});
    // bore
    Op bo; bo.type = OpType::Bore; bo.name = "Bore"; bo.depth = 0.4; bo.stepdown = 0.1;
    add("bore_hole", bo, em, {circ(2, 2, 0.5)});
    add("bore_small", bo, em, {circ(2, 2, 0.1)});
    { Op b = bo; b.depth = 0.33; b.stepdown = 0.12; b.allowance = 0.02; add("bore_two_odd_pitch", b, em, {circ(2, 2, 0.4), circ(5, 2, 0.7)}); }
    // inlay
    Op il; il.type = OpType::Inlay; il.name = "Inlay"; il.tool = 2; il.flatDepth = 0.2; il.glueGap = 0.1; il.inlayMargin = 0.5; il.clearTool = 0;
    add("inlay_square", il, vb, {makeRect(1, 1, 2, 2)}, &tools[0]);
    add("inlay_star_no_clear", il, tools[2], {star()});
    { Op b = il; b.glueGap = 0.3; add("inlay_bad_gap", b, vb, {makeRect(1, 1, 2, 2)}); b.flatDepth = 0; add("inlay_no_flat", b, vb, {makeRect(1, 1, 2, 2)}); }
    // flute
    Op fl; fl.type = OpType::Flute; fl.name = "Flute"; fl.depth = 0.2; fl.stepdown = 0.1; fl.rampIn = 1; fl.rampOut = 1;
    add("flute_line", fl, em, {makePolyline({{0, 0}, {4, 0}}, false)});
    { Op l = fl; l.rampIn = l.rampOut = 10; l.stepdown = 0.2; add("flute_lens", l, em, {makePolyline({{0, 0}, {4, 0}}, false)}); }
    { Op l = fl; l.rampIn = 0.3; l.rampOut = 0; l.depth = 0.25; add("flute_curves", l, em, {makeEllipse(3, 3, 1, 0.5), letters()[3]}); }

    // chamfer
    Op ch; ch.type = OpType::Chamfer; ch.name = "Chamfer"; ch.tool = 2; ch.side = Side::Inside; ch.chamferWidth = 0.1;
    add("chamfer_rect_inside", ch, vb, {makeRect(1, 1, 3, 2)});
    { Op o = ch; o.side = Side::Outside; o.climb = false; add("chamfer_rect_outside_conventional", o, vb, {makeRect(1, 1, 3, 2)}); }
    { Op o = ch; o.side = Side::On; add("chamfer_letters_on", o, tools[2], letters()); }
    add("chamfer_disc_with_hole", ch, vb, {circ(c.x, c.y, 0.925), circ(c.x, c.y, 0.458333)});
    add("chamfer_star_sharp", ch, tools[2], {star()}, nullptr, Material{"Plywood", 12, 8, 0.75});
    { Op o = ch; o.chamferWidth = 0; o.depth = 0.1; o.startDepth = 0.05; o.stepdown = 0.04; add("chamfer_open_by_depth_passes", o, vb, {makePolyline({{0, 0}, {2, 0.5}, {3, 2}}, false)}); }
    { Op o = ch; o.chamferWidth = 0.4; add("chamfer_too_wide_for_bit", o, tools[2], {makeRect(1, 1, 3, 2)}); }
    add("chamfer_endmill_refused", ch, em, {makeRect(1, 1, 3, 2)});
    // prism
    Op pr; pr.type = OpType::Prism; pr.name = "Prism"; pr.tool = 2; pr.depth = 0.4;
    add("prism_bar", pr, vb, {makeRect(1, 1, 3, 1.4)});
    { Op o = pr; o.depth = 0.1; add("prism_bar_capped_flat_top", o, vb, {makeRect(1, 1, 3, 1.4)}); }
    add("prism_neighbours", pr, vb, {makeRect(1, 1, 3, 1.4), makeRect(1, 1.5, 3, 1.9)});
    { Op o = pr; o.depth = 0.25; add("prism_letters", o, tools[2], letters()); o.stepdown = 0.06; add("prism_letters_layers", o, tools[2], letters()); }
    add("prism_star", pr, tools[2], {star()}, nullptr, Material{"Plywood", 12, 8, 0.75});
    add("prism_endmill_refused", pr, em, {makeRect(1, 1, 3, 1.4)});
    add("prism_open_skipped", pr, vb, {makePolyline({{0, 0}, {1, 1}}, false)});

    std::string j = "[\n";
    for (size_t k = 0; k < cases.size(); ++k) {
        const Case& cs = cases[k];
        CamResult r = generate(cs.op, cs.tool, cs.cs, cs.m, cs.clear);
        CamResult raw;
        switch (cs.op.type) {
        case OpType::Pocket: raw = generatePocket(cs.op, cs.tool, cs.cs, cs.m); break;
        case OpType::Drill: raw = generateDrill(cs.op, cs.tool, cs.cs, cs.m); break;
        case OpType::VCarve: raw = generateVCarve(cs.op, cs.tool, cs.cs, cs.m, cs.clear); break;
        case OpType::Bore: raw = generateBore(cs.op, cs.tool, cs.cs, cs.m); break;
        case OpType::Inlay: raw = generateInlay(cs.op, cs.tool, cs.cs, cs.m, cs.clear); break;
        case OpType::Flute: raw = generateFlute(cs.op, cs.tool, cs.cs, cs.m); break;
        case OpType::Chamfer: raw = generateChamfer(cs.op, cs.tool, cs.cs, cs.m); break;
        case OpType::Prism: raw = generatePrism(cs.op, cs.tool, cs.cs, cs.m); break;
        default: raw = generateProfile(cs.op, cs.tool, cs.cs, cs.m); break;
        }
        // jobs + safety for every case (tools: the case tool at the op's index, clearance tool at its index)
        std::vector<Tool> jt = tools;
        if (cs.op.tool >= 0 && cs.op.tool < int(jt.size())) jt[cs.op.tool] = cs.tool;
        if (cs.clear && cs.op.clearTool >= 0) jt[cs.op.clearTool] = *cs.clear;
        auto jobs = buildJobs({cs.op}, jt, {r.paths}, cs.m, "t");
        std::vector<std::string> taps; for (auto& jb : jobs) taps.push_back(postWinCNC(jb));
        j += "{\"name\":" + str(cs.name) + ",\"op\":" + jOp(cs.op) + ",\"tool\":" + jTool(cs.tool) + ",\"clear_tool\":" + (cs.clear ? jTool(*cs.clear) : "null") +
             ",\"tools\":" + arr(jt, jTool) + ",\"material\":" + jMat(cs.m) + ",\"contours\":" + arr(cs.cs, jContour) +
             ",\"raw\":" + arr(raw.paths, jPath) + ",\"raw_warnings\":" + jStrs(raw.warnings) +
             ",\"paths\":" + arr(r.paths, jPath) + ",\"warnings\":" + jStrs(r.warnings) +
             ",\"safety\":" + jStrs(safetyIssues({cs.op}, jt, {r.paths}, cs.m)) + ",\"taps\":" + jStrs(taps) + "}" + (k + 1 < cases.size() ? ",\n" : "\n");
    }
    j += "]\n";
    std::ofstream(outDir + "cases.json", std::ios::binary) << j;

    // ---- samples/compare: the whole pipeline (DXF -> ops per layer -> generate -> buildJobs -> WinCNC), compare.js settings
    {
        std::ifstream f(root + "/samples/compare/compare.dxf", std::ios::binary);
        std::stringstream ss; ss << f.rdbuf();
        ImportResult ir = importDxf(ss.str());
        Material m; m.type = "Plywood"; m.w = 12; m.h = 8; m.t = 0.75; m.autoFeeds = false; m.safeZ = 0.8; m.retractZ = 0.2;
        std::vector<Tool> ct = defaultTools();
        ct[0] = {"1/4 End Mill", ToolType::EndMill, 0.25, 0, 0.25, 432, 216, 18000};
        ct[2] = {"V-Bit (90 deg 0.5in)", ToolType::VBit, 0.5, 90, 0.25, 100, 50, 18000};
        Op p; p.name = "Profile"; p.tool = 0; p.side = Side::Outside; p.depth = 0.75; p.stepdown = 0.25; p.entry = Entry::Plunge; p.climb = true;
        p.tabs = true; p.tabCount = 4; p.tabWidth = 0.5; p.tabHeight = 0.125; p.tabRamped = false;
        Op q; q.type = OpType::Pocket; q.name = "Pocket"; q.tool = 0; q.depth = 0.25; q.stepdown = 0.25; q.stepover = 0.4; q.entry = Entry::Plunge; q.climb = true;
        Op d; d.type = OpType::Drill; d.name = "Drill"; d.tool = 0; d.depth = 0.5; d.peck = 0.25;
        Op v; v.type = OpType::VCarve; v.name = "VCarve"; v.tool = 2; v.flatDepth = 0; v.clearTool = -1;
        std::vector<Op> ops{p, q, d, v};
        const char* layer[] = {"1_PROFILE_OUT", "2_POCKET", "3_DRILL", "4_VCARVE"};
        std::vector<std::vector<Toolpath>> gen;
        std::string jc = "{\"material\":" + jMat(m) + ",\"tools\":" + arr(ct, jTool) + ",\"ops\":" + arr(ops, jOp) + ",\"layers\":[";
        for (int k = 0; k < 4; ++k) {
            std::vector<Contour> cs;
            for (size_t i = 0; i < ir.contours.size(); ++i) if (ir.layers[i] == layer[k]) cs.push_back(ir.contours[i]);
            jc += std::string(k ? "," : "") + arr(cs, jContour);
            gen.push_back(generate(ops[k], ct[ops[k].tool], cs, m, nullptr).paths);
        }
        auto jobs = buildJobs(ops, ct, gen, m, "09:09 PM, Tuesday December 17 2024");
        std::vector<std::string> taps; for (auto& jb : jobs) taps.push_back(postWinCNC(jb));
        auto air = jobs; makeAirCut(air, airCutRaise(m));
        std::vector<std::string> airTaps; for (auto& jb : air) airTaps.push_back(postWinCNC(jb));
        Material tbl = m; tbl.zBottom = true; tbl.origin = 4;
        auto tjobs = buildJobs(ops, ct, gen, tbl, "t");
        std::vector<std::string> tblTaps; for (auto& jb : tjobs) tblTaps.push_back(postWinCNC(jb));
        jc += "],\"generated\":" + arr(gen, [](const std::vector<Toolpath>& v) { return arr(v, jPath); }) + ",\"taps\":" + jStrs(taps) + ",\"aircut_taps\":" + jStrs(airTaps) +
              ",\"table_centre_taps\":" + jStrs(tblTaps) + ",\"safety\":" + jStrs(safetyIssues(ops, ct, gen, m)) + "}\n";
        std::ofstream(outDir + "compare.json", std::ios::binary) << jc;
    }

    // ---- relief (samples/pyramid.stl) + simulation
    {
        std::vector<Triangle> tris; std::string err;
        if (!loadStl(root + "/samples/pyramid.stl", tris, &err)) { std::fprintf(stderr, "%s\n", err.c_str()); return 1; }
        Relief r; r.x = 1; r.y = 1; r.w = 2; r.h = 2; r.depth = 0.3;
        rasterizeRelief(tris, r, 40000);
        Relief rm = r; rm.mirror = true; rm.w = 3; rasterizeRelief(tris, rm, 5000);
        Tool ball; ball.type = ToolType::BallNose; ball.diameter = 0.125; ball.feedXY = 100; ball.feedZ = 50;
        Tool flat; flat.diameter = 0.25;
        Tool vee = tools[2];
        Op ro; ro.type = OpType::Relief; ro.name = "relief"; ro.stepover = 0.2; ro.rough = true; ro.roughStepdown = 0.1; ro.roughStepover = 0.5; ro.allowance = 0.02; ro.clearTool = 0;
        Material m; m.w = 4; m.h = 4; m.t = 0.75;
        std::vector<Contour> bnd{makeEllipse(2, 2, 0.6, 0.6)};
        Op ry = ro; ry.rasterAngle = 90; ry.rough = false; ry.stepover = 0.5;
        auto a = generateRelief(ro, ball, &flat, r, m);
        auto b = generateRelief(ro, ball, nullptr, r, m, &bnd);
        auto cc = generateRelief(ry, vee, nullptr, rm, m);
        std::vector<double> tips;
        for (double x = 0.8; x <= 3.2; x += 0.3) for (double y = 0.9; y <= 3.1; y += 0.35)
            for (const Tool* t : {&ball, &flat, &vee}) tips.push_back(tipHeight(r, *t, x, y, 0.01));
        auto hs = [](const Relief& rr) { return arr(rr.height, [](float f) { return num(f); }); };
        std::string jr = "{\"triangles\":" + std::to_string(tris.size()) + ",\"nx\":" + std::to_string(r.nx) + ",\"ny\":" + std::to_string(r.ny) + ",\"height\":" + hs(r) +
                         ",\"mirror_nx\":" + std::to_string(rm.nx) + ",\"mirror_ny\":" + std::to_string(rm.ny) + ",\"mirror_height\":" + hs(rm) +
                         ",\"tips\":" + arr(tips, num) + ",\"ball\":" + jTool(ball) + ",\"flat\":" + jTool(flat) + ",\"vee\":" + jTool(vee) + ",\"op\":" + jOp(ro) + ",\"op_y\":" + jOp(ry) +
                         ",\"material\":" + jMat(m) + ",\"boundary\":" + arr(bnd, jContour) +
                         ",\"rough_finish\":" + arr(a.paths, jPath) + ",\"bounded\":" + arr(b.paths, jPath) + ",\"vee_y_mirror\":" + arr(cc.paths, jPath) + ",\"warnings\":" + jStrs(a.warnings);

        // simulation: pocket + profile + v-carve on a small board, ~20000 cells
        Material sm{"P", 4, 3, 0.5};
        Op pk2; pk2.type = OpType::Pocket; pk2.depth = 0.2;
        CamResult pocket = generatePocket(pk2, em, {makeRect(1, 1, 3, 2)}, sm);
        Op pr2; pr2.depth = 0.5; pr2.entry = Entry::Plunge;
        CamResult prof = generateProfile(pr2, em, {makeEllipse(0.6, 0.6, 0.25, 0.25)}, sm);
        Op v2; v2.type = OpType::VCarve; v2.tool = 2;
        CamResult vc2 = generateVCarve(v2, vb, {makeRect(0.5, 0.5, 1.5, 0.9)}, sm);
        Tool bn = tools[3];
        HeightField hf = simulate({{&pocket.paths[0], &em}, {&prof.paths[0], &em}, {&vc2.paths[0], &vb}, {&a.paths[1], &bn}}, sm, 20000);
        std::vector<unsigned char> img = shade(hf, sm);
        std::vector<double> imgD(img.begin(), img.end());
        jr += ",\"sim_material\":" + jMat(sm) + ",\"sim_paths\":[" + jPath(pocket.paths[0]) + "," + jPath(prof.paths[0]) + "," + jPath(vc2.paths[0]) + "," + jPath(a.paths[1]) + "]" +
              ",\"sim_tools\":[" + jTool(em) + "," + jTool(em) + "," + jTool(vb) + "," + jTool(bn) + "]" +
              ",\"sim\":{\"nx\":" + std::to_string(hf.nx) + ",\"ny\":" + std::to_string(hf.ny) + ",\"res\":" + num(hf.res) + ",\"x0\":" + num(hf.x0) + ",\"y0\":" + num(hf.y0) +
              ",\"depth\":" + arr(hf.depth, [](float f) { return num(f); }) + ",\"img\":" + arr(imgD, [](double d) { return std::to_string(int(d)); }) + "}}\n";
        std::ofstream(outDir + "relief_sim.json", std::ios::binary) << jr;
    }
    std::printf("wrote %zu cases\n", cases.size());
    return 0;
}
