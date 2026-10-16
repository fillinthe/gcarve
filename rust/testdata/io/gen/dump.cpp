// Golden generator for the Rust port of the importers, vector ops and nesting (rust/gcarve-core/tests/io_golden.rs).
// Runs the C++ engine and prints JSON. Rebuild + regenerate from the repo root:
//   c++ -std=c++20 -O1 -Icore -Ibuild/_deps/clipper2-src/CPP/Clipper2Lib/include rust/testdata/io/gen/dump.cpp \
//       build/core/libgcarve_core.a build/_deps/clipper2-build/libClipper2.a -o /tmp/io_dump
//   /tmp/io_dump . > rust/testdata/io/golden.json
#include <cstdio>
#include <fstream>
#include <sstream>
#include <string>

#include "import.h"
#include "nest.h"
#include "vecops.h"

using namespace gcarve;

static std::string out;
static void num(double v) { char b[40]; std::snprintf(b, sizeof b, "%.17g", v); out += b; }
static void pt(Pt p) { out += "["; num(p.x); out += ","; num(p.y); out += "]"; }
static void contour(const Contour& c) {
    out += "{\"closed\":"; out += c.closed ? "true" : "false"; out += ",\"nodes\":[";
    for (size_t i = 0; i < c.nodes.size(); ++i) {
        const Node& n = c.nodes[i];
        if (i) out += ",";
        out += "{\"p\":"; pt(n.p);
        if (n.hasIn) { out += ",\"in\":"; pt(n.in); }
        if (n.hasOut) { out += ",\"out\":"; pt(n.out); }
        out += "}";
    }
    out += "]}";
}
static void contours(const std::vector<Contour>& cs) {
    out += "[";
    for (size_t i = 0; i < cs.size(); ++i) { if (i) out += ","; contour(cs[i]); }
    out += "]";
}
static void str(const std::string& s) {
    out += "\"";
    for (char c : s) { if (c == '"' || c == '\\') out += '\\'; out += c; }
    out += "\"";
}
static bool first = true;
static void key(const std::string& k) { out += first ? "\n" : ",\n"; first = false; str(k); out += ":"; }
static void imported(const std::string& name, const ImportResult& r) {
    key(name);
    out += "{\"note\":"; str(r.note); out += ",\"layers\":[";
    for (size_t i = 0; i < r.layers.size(); ++i) { if (i) out += ","; str(r.layers[i]); }
    out += "],\"contours\":"; contours(r.contours); out += "}";
}
static std::string read(const std::string& p) { std::ifstream f(p, std::ios::binary); std::stringstream s; s << f.rdbuf(); return s.str(); }

int main(int argc, char** argv) {
    std::string root = argc > 1 ? argv[1] : ".";
    out = "{";
    imported("import:samples/demo.svg", importSvg(read(root + "/samples/demo.svg")));
    imported("import:samples/demo.dxf", importDxf(read(root + "/samples/demo.dxf")));
    imported("import:samples/compare/compare.dxf", importDxf(read(root + "/samples/compare/compare.dxf")));
    imported("import:inline.svg", importSvg(R"svg(<?xml version="1.0"?>
      <svg xmlns="http://www.w3.org/2000/svg" xmlns:inkscape="x" width="25.4mm" height="25.4mm" viewBox="0 0 100 100">
        <!-- comment <rect/> -->
        <g transform="translate(10,10)" inkscape:label="Cut"><rect x="0" y="0" width="20" height="10"/></g>
        <g id="g123" transform="rotate(30 50 50) scale(0.5)"><circle cx="50" cy="50" r="10"/><ellipse cx="20" cy="30" rx="8" ry="3"/></g>
        <path d="M 60 60 h 10 v 10 h -10 z m 5 5 l 2 0 0 2 z"/>
        <path d="M10 80 C 20 70 30 90 40 80 S 60 70 70 80 Q 80 90 90 80 T 95 95 L 97 97"/>
        <path d="M5 5 A 10 5 30 1 0 25 15 a 3 3 0 0 1 6 0"/>
        <rect x="70" y="5" width="20" height="15" rx="4"/>
        <polygon points="1,1 9,1 5,9"/><polyline points="20 1 25 4 30 1"/><line x1="0" y1="99" x2="99" y2="99"/>
        <g transform="matrix(1 0 0.2 1 0 0) skewX(5)"><rect x="40" y="40" width="5" height="5"/></g>
        <text>hi</text><use href="#a"/>
        <defs><rect width="999" height="999"/></defs>
        <rect x="0" y="0" width="30" height="30" rx="5" display="none"/>
      </svg>)svg"));
    imported("import:inline.dxf", importDxf("0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n"
                         "0\nLWPOLYLINE\n8\nCUT\n90\n2\n70\n1\n10\n0\n20\n0\n42\n1\n10\n25.4\n20\n0\n"
                         "0\nLINE\n10\n0\n20\n50.8\n11\n25.4\n21\n50.8\n"
                         "0\nLINE\n10\n25.4\n20\n50.8\n11\n25.4\n21\n76.2\n"
                         "0\nCIRCLE\n8\n0\n10\n0\n20\n0\n40\n12.7\n"
                         "0\nARC\n10\n100\n20\n0\n40\n10\n50\n300\n51\n45\n230\n-1\n"
                         "0\nPOLYLINE\n70\n1\n0\nVERTEX\n10\n200\n20\n0\n42\n-0.5\n0\nVERTEX\n10\n220\n20\n0\n0\nVERTEX\n10\n220\n20\n20\n0\nSEQEND\n"
                         "0\nSPLINE\n0\nTEXT\n0\nTEXT\n0\nENDSEC\n0\nEOF\n"));

    Contour rect = makeRect(0, 0, 2, 2), rect2 = makeRect(1, 1, 3, 3), circ = makeEllipse(2, 1, 1.2, 0.8);
    Contour L = makePolyline({{0, 0}, {4, 0}, {4, 1}, {1, 1}, {1, 3}, {0, 3}}, true);
    Contour star = makeStar({1, 1}, 1, 0.4, 5, 0.1);
    Contour open = makePolyline({{0, 0}, {2, 0}, {2, 1}}, false);
    key("weld"); contours(booleanOp(BoolOp::Weld, {rect}, {rect2, circ}));
    key("subtract"); contours(booleanOp(BoolOp::Subtract, {makeRect(0, 0, 3, 3)}, {makeRect(1, 1, 2, 2), circ}));
    key("intersect"); contours(booleanOp(BoolOp::Intersect, {rect}, {circ}));
    key("xor"); contours(booleanOp(BoolOp::Xor, {rect}, {rect2}));
    key("offset+0.5"); contours(offsetContours({rect, circ}, 0.5));
    key("offset-0.3"); contours(offsetContours({L}, -0.3));
    key("offset-miter"); contours(offsetContours({star}, 0.1, false));
    key("stroke-round"); contours(offsetContours({open}, 0.25));
    key("stroke-square"); contours(offsetContours({open}, -0.25, false));
    key("dogbone-in"); contour(dogbone(makeRect(0, 0, 4, 2), 0.1255, true, false));
    key("tbone-in"); contour(dogbone(makeRect(0, 0, 4, 2), 0.1255, true, true));
    key("dogbone-L-out"); contour(dogbone(L, 0.125, false, false));
    key("tbone-L-out"); contour(dogbone(L, 0.125, false, true));
    key("polygon"); contour(makePolygon({1, 2}, 1.5, 7, 0.3));
    key("star"); contour(star);
    key("close"); contour(closeContour(open));
    key("join"); contours(joinOpen({makePolyline({{0, 0}, {1, 0}}, false), makePolyline({{1, 1}, {1, 0}}, false), makePolyline({{1, 1}, {0, 0}}, false)}, 1e-4));

    key("nestBoxes");
    out += "[";
    auto boxes = nestBoxes({{3, 2}, {3, 2}, {3, 2}, {2, 6}, {5, 1}, {1, 1}, {4, 3}}, 18, 8, 0.25, 0.5, true);
    for (size_t i = 0; i < boxes.size(); ++i) {
        if (i) out += ",";
        out += "{\"x\":"; num(boxes[i].x); out += ",\"y\":"; num(boxes[i].y);
        out += ",\"rotated\":"; out += boxes[i].rotated ? "true" : "false"; out += ",\"placed\":"; out += boxes[i].placed ? "true" : "false"; out += "}";
    }
    out += "]";
    key("nestShapes");
    out += "[";
    std::vector<NestPart> parts{{{{0, 0}, {4, 0}, {0, 4}}}, {{{0, 0}, {4, 0}, {0, 4}}}, {{{0, 0}, {2, 0}, {2, 1}, {0, 1}}}, {{{0, 0}, {1, 0}, {1, 3}, {0, 3}}}, {{{0, 0}, {9, 0}, {9, 9}}}};
    auto pl = nestShapes(parts, 8, 7, 0.2, 0.5, 90);
    for (size_t i = 0; i < pl.size(); ++i) {
        if (i) out += ",";
        out += "{\"placed\":"; out += pl[i].placed ? "true" : "false"; out += ",\"rotation\":"; num(pl[i].rotation);
        out += ",\"offset\":"; pt(pl[i].offset); out += "}";
    }
    out += "]\n}\n";
    std::fputs(out.c_str(), stdout);
}
