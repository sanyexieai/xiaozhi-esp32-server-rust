import sqlite3
import json

db = sqlite3.connect("data/manager.db")
out = {"agents": [], "imported": [], "mcp_servers": []}

for row in db.execute("SELECT id, name, extra_json FROM agents"):
    out["agents"].append({"id": row[0], "name": row[1], "extra": json.loads(row[2] or "{}")})

for row in db.execute("SELECT name, json_data FROM configs WHERE type='mcp_imported'"):
    out["imported"].append({"name": row[0], "data": json.loads(row[1])})

for row in db.execute("SELECT json_data FROM configs WHERE type='mcp' LIMIT 1"):
    j = json.loads(row[0])
    block = j.get("mcp", j)
    for s in block.get("global", {}).get("servers", []):
        out["mcp_servers"].append(s)

for row in db.execute("SELECT id, device_id, agent_id, name FROM devices WHERE device_id LIKE 'web-sim-%'"):
    out.setdefault("sim_devices", []).append(
        {"id": row[0], "device_id": row[1], "agent_id": row[2], "name": row[3]}
    )

with open("scripts/mcp_dump.json", "w", encoding="utf-8") as f:
    json.dump(out, f, ensure_ascii=False, indent=2)
print("written scripts/mcp_dump.json")
