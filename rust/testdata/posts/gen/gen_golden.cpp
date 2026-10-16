// Writes the golden files the Rust port is checked against (rust/gcarve-core/tests/golden_cpp.rs):
//   <id>_<n>.<ext>                 postJobs output of every built-in post for the fixture jobs below (index.txt lists them)
//   pp-<name>_<n>.<ext>, <name>.json   the same for the two Vectric .pp test posts, plus their postToJson definition
//   arcfit_in.txt / arcfit_out.txt     fitArcs input and output toolpaths
//   gcode_*.txt                        parseProgram dumps of samples/compare/grbl/*.nc, reference/*.tap and misc.nc
// Build + run: rust/testdata/posts/gen/build.sh (compiles the C++ sources directly, so it always matches the tree).
#include <cmath>
#include <cstdio>
#include <fstream>
#include <sstream>

#include "arcfit.h"
#include "gcode.h"
#include "postdef.h"

using namespace gcarve;

static Move M(MoveType t, double x, double y, double z, double i = 0, double j = 0) {
    Move m; m.type = t;
    if (!std::isnan(x)) m.x = x;
    if (!std::isnan(y)) m.y = y;
    if (!std::isnan(z)) m.z = z;
    m.i = i; m.j = j; return m;
}
static const double N = NAN;
static const MoveType R = MoveType::Rapid, L = MoveType::Linear, CW = MoveType::ArcCW, CCW = MoveType::ArcCCW;

// Keep in sync with fixture_jobs() in rust/gcarve-core/tests/golden_cpp.rs
static std::vector<Job> fixtureJobs() {
    Job a; a.filename = "01_Profile_(outside)"; a.timestamp = "09:09 PM, Tuesday December 17 2024"; a.material = "48.000\" x 24.000\" x 0.750\"";
    a.tool = "1/4 End Mill (0.25 inch)"; a.rpm = 18000; a.notes = {"AIR CUT TEST - NOT FOR CUTTING"};
    Toolpath p1; p1.name = "Profile 1 (outside)"; p1.feedXY = 432; p1.feedZ = 216;
    p1.moves = {M(R, N, N, 0.8), M(R, 1, 1, N), M(L, N, N, -0.25), M(L, 3, N, N), M(L, 3, 2, -0.25), M(CCW, 1, 2, N, -1, 0), M(CW, 2, 3, N, 1, 0),
                M(CCW, 2, 3, N, 0, -1), M(L, N, N, -0.5), M(L, 2.5, 3.2, -0.5), M(L, 2.123456789, 3.987654321, -0.5), M(R, N, N, 0.8)};
    Toolpath p2; p2.name = "Pocket inner"; p2.feedXY = 300; p2.feedZ = 100;
    p2.moves = {M(R, 5, 5, N), M(L, N, N, -0.1), M(L, 6, 5, N), M(L, N, 6, N), M(CW, 5, 6, -0.2, -0.5, 0), M(L, 5, 5, -0.2), M(R, N, N, 0.8)};
    a.toolpaths = {p1, p2};
    Job b; b.filename = "02_V-Bit"; b.timestamp = a.timestamp; b.material = a.material; b.tool = "V-Bit (90 deg 0.5\")"; b.rpm = 16000; b.zZero = "Table";
    b.notes = {"AIR CUT TEST - NOT FOR CUTTING"};
    Toolpath v; v.name = "V-Carve"; v.feedXY = 120.5; v.feedZ = 60.25;
    v.moves = {M(R, 0, 0, 0.8), M(L, N, N, -0.1), M(L, 0.333333, 0.1, -0.15), M(L, 1, 0, N), M(L, 1, 0, -0.1), M(R, N, N, 0.5), M(R, 10, 10, 0.5), M(L, 10, 10, -0.05)};
    Toolpath e; e.name = "Empty"; e.feedXY = 50; e.feedZ = 25;
    b.toolpaths = {v, e};
    return {a, b};
}

static std::string root;
static void put(const std::string& path, const std::string& text) { std::ofstream(root + path, std::ios::binary) << text; }
static std::string slurp(const std::string& path) { std::ifstream f(path, std::ios::binary); std::stringstream s; s << f.rdbuf(); return s.str(); }
static std::string g(double v) { char b[64]; std::snprintf(b, sizeof b, "%.17g", v); return b; }
static std::string opt(const std::optional<double>& v) { return v ? g(*v) : "-"; }
static std::string dumpTp(const Toolpath& tp) {
    std::string o = "toolpath " + std::to_string(tp.moves.size()) + "\n";
    for (const Move& m : tp.moves) o += std::to_string(int(m.type)) + " " + opt(m.x) + " " + opt(m.y) + " " + opt(m.z) + " " + g(m.i) + " " + g(m.j) + "\n";
    return o;
}

int main(int argc, char** argv) {
    root = std::string(argc > 1 ? argv[1] : ".") + "/";   // rust/testdata/posts/
    std::string repo = root + "../../../";
    auto jobs = fixtureJobs();
    std::string index;
    auto emit = [&](const std::string& id, const std::vector<PostOutput>& outs) {
        int n = 0;
        for (const PostOutput& po : outs) {
            std::string f = id + "_" + std::to_string(n++) + "." + po.filename.substr(po.filename.rfind('.') + 1);
            put(f, po.text);
            index += id + " " + f + " " + po.filename + "\n";
        }
    };
    for (const PostDef& d : postLibrary()) emit(d.id, postJobs(jobs, d));
    for (const char* pp : {"test-router", "mach3-like"}) {
        std::string err;
        PostDef d = postFromVectricPP(slurp(root + pp + ".pp"), &err);
        put(std::string(pp) + ".json", postToJson(d));
        emit(std::string("pp-") + pp, postJobs(jobs, d));
    }
    put("index.txt", index);

    // arcfit: flattened circles / arcs / rounded rectangle / helix
    std::vector<Toolpath> ins;
    auto circle = [&](double cx, double cy, double r, int n, double z, bool ccw, double a0, double sweep) {
        Toolpath t; t.name = "c";
        t.moves.push_back(M(R, cx + r * std::cos(a0), cy + r * std::sin(a0), 0.5));
        t.moves.push_back(M(L, N, N, z));
        for (int k = 1; k <= n; ++k) { double a = a0 + (ccw ? 1 : -1) * sweep * k / n; t.moves.push_back(M(L, cx + r * std::cos(a), cy + r * std::sin(a), N)); }
        t.moves.push_back(M(R, N, N, 0.5));
        return t;
    };
    ins.push_back(circle(3, 3, 1.05, 72, -0.484, true, 0, 2 * M_PI));
    ins.push_back(circle(0, 0, 0.5, 30, -0.1, false, 0.3, 2 * M_PI / 3));
    ins.push_back(circle(1, 1, 0.005, 12, -0.1, true, 0, M_PI));          // radius below the 0.01 limit
    ins.push_back(circle(-2, 5, 12.5, 200, -0.2, true, 1, 2 * M_PI));
    {   // rounded rectangle: 4 straight sides + quarter-circle corners of radius 0.125
        Toolpath t; t.name = "rr";
        t.moves.push_back(M(R, 1, 0.875, 0.5)); t.moves.push_back(M(L, N, N, -0.25));
        double cs[4][3] = {{1, 1, 1.5 * M_PI}, {1, 7, M_PI}, {11, 7, 0.5 * M_PI}, {11, 1, 0}};
        for (auto& c : cs)
            for (int k = 0; k <= 9; ++k) { double a = c[2] - 0.5 * M_PI * k / 9; t.moves.push_back(M(L, c[0] + 0.125 * std::cos(a), c[1] + 0.125 * std::sin(a), k == 0 ? -0.25 : N)); }
        t.moves.push_back(M(L, 1, 0.875, -0.25));
        ins.push_back(t);
    }
    {   // helix: z changes every move -> no arcs; then a flat lap with explicit z
        Toolpath t; t.name = "helix";
        for (int k = 0; k <= 72; ++k) { double a = 2 * M_PI * k / 36; t.moves.push_back(M(L, 2 + 0.375 * std::cos(a), 2 + 0.375 * std::sin(a), -0.4 * k / 72)); }
        for (int k = 1; k <= 36; ++k) { double a = 2 * M_PI * k / 36; t.moves.push_back(M(L, 2 + 0.375 * std::cos(a), 2 + 0.375 * std::sin(a), -0.4)); }
        ins.push_back(t);
    }
    std::string fin, fout;
    for (const Toolpath& t : ins) { fin += dumpTp(t); fout += dumpTp(fitArcs(t)); fout += dumpTp(fitArcs(t, 0.0001, M_PI)); }
    put("arcfit_in.txt", fin);
    put("arcfit_out.txt", fout);

    // gcode reader
    auto dumpProg = [&](const std::string& name, const std::string& text) {
        GProgram p = parseProgram(text);
        std::string o = "lines " + std::to_string(p.lines.size()) + "\nerrors " + std::to_string(p.errors.size()) + "\n";
        for (auto& e : p.errors) o += e + "\n";
        o += "motions " + std::to_string(p.motions.size()) + "\n";
        for (const GMotion& m : p.motions)
            o += std::to_string(int(m.type)) + " " + g(m.from.x) + " " + g(m.from.y) + " " + g(m.from.z) + " " + g(m.to.x) + " " + g(m.to.y) + " " + g(m.to.z) + " " + g(m.i) + " " + g(m.j) + " " +
                 g(m.feed) + " " + std::to_string(m.machineCoords) + " " + std::to_string(m.spindleOn) + " " + std::to_string(m.line) + " " + std::to_string(motionPoints(m).size()) + "\n";
        o += "min " + g(p.min.x) + " " + g(p.min.y) + " " + g(p.min.z) + "\nmax " + g(p.max.x) + " " + g(p.max.y) + " " + g(p.max.z) + "\n";
        o += "spindleOffAtEnd " + std::to_string(p.spindleOffAtEnd) + "\nminutes " + g(p.minutes) + "\n";
        put("gcode_" + name + ".txt", o);
    };
    for (const char* f : {"samples/compare/grbl/compare_01_1_4_End_Mill.nc", "samples/compare/grbl/compare_02_V_Bit_90_deg_0_5in.nc", "reference/01_V90.tap", "reference/02_EMD14.tap"}) {
        std::string s = f; s = s.substr(s.rfind('/') + 1); s = s.substr(0, s.rfind('.'));
        dumpProg(s, slurp(repo + f));
    }
    dumpProg("misc", slurp(root + "misc.nc"));
    return 0;
}
