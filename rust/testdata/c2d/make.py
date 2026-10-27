# Writes small.c2d: a Carbide Create version 7 style database (our own content) for the c2d page reader test.
# python3 -I make.py  (in this folder)
import json, math, os, sqlite3, zlib

out = os.path.join(os.path.dirname(os.path.abspath(__file__)), "small.c2d")
if os.path.exists(out):
    os.remove(out)
db = sqlite3.connect(out)
db.execute("create table items(id integer primary key autoincrement, uuid text unique, name text, type text, version text, sz int, data blob)")
db.execute("create table params(key text, value text)")
for k, v in [("width", "304.8"), ("height", "203.2"), ("thickness", "19.05"), ("retract", "6.35"), ("display_mm", "0"), ("build_num", "853")]:
    db.execute("insert into params values (?, ?)", (k, v))

def put(uuid, typ, obj):
    raw = json.dumps(obj).encode()
    db.execute("insert into items (uuid, name, type, version, sz, data) values (?, ?, ?, 'J1', ?, ?)", (uuid, typ, typ, len(raw), zlib.compress(raw)))

layer = {"name": "Parts", "visible": True}
# a square by cubics (straight ones)
sq = [[25.4, 25.4], [127, 25.4], [127, 127], [25.4, 127], [25.4, 25.4]]
put("{e1}", "element", {"geometryType": "path", "id": "{e1}", "layer": layer, "position": [0, 0], "points": sq + [[0, 0]], "cp1": sq + [[0, 0]], "cp2": sq + [[0, 0]], "point_type": [0, 3, 3, 3, 3, 4]})
# a wobbly ring of 600 points: its zlib data is bigger than a page
ring = [[200 + (40 + 3 * math.sin(i * 1.7) + 2 * math.cos(i * 0.37)) * math.cos(i * 2 * math.pi / 600), 100 + (40 + 3 * math.sin(i * 1.3)) * math.sin(i * 2 * math.pi / 600)] for i in range(600)]
ring = [[round(x, 6), round(y, 6)] for x, y in ring]
put("{e2}", "element", {"geometryType": "path", "id": "{e2}", "layer": layer, "position": [0, 0], "points": ring + [ring[0]], "cp1": ring + [ring[0]], "cp2": ring + [ring[0]], "point_type": [0] + [1] * 599 + [4]})
tool = {"diameter": 6.35, "angle": 0, "corner_radius": 0, "flutes": 2, "name": "1/4"}
speeds = {"feedrate": 1524, "plungerate": 381, "rpm": 18000}
put("{t1}", "toolpath", {"type": "contour", "name": "Cut out", "ofset_dir": 1, "enabled": True, "end_depth": "20.955", "stepdown": 3.175, "tool": tool, "speeds": speeds, "elements": [{"uuid": "{e1}"}]})
put("{t2}", "toolpath", {"type": "pocket_toolpath", "name": "Pocket", "enabled": True, "end_depth": "1/4", "stepdown": 3.175, "stepover": 3.175, "tool": tool, "speeds": speeds, "elements": [{"uuid": "{e2}"}]})
db.commit()
db.close()
print(out, os.path.getsize(out))
