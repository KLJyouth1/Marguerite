"""Dump plan card payloads from the sidecar state store for diagnosis."""
import json
import os
import sqlite3

TARGET = "rev-573410db-428b-40d5-b1e9-aaee0a7881f3"

DB = os.path.expandvars(r"%LOCALAPPDATA%\Marguerite\pyengine-home\default\state\sessions.db")
print("DB:", DB)
con = sqlite3.connect(DB)
cur = con.cursor()
cur.execute("PRAGMA table_info(transcript_entries)")
print("transcript_entries columns:", [r[1] for r in cur.fetchall()])

cur.execute("SELECT COUNT(*) FROM transcript_entries")
print("transcript_entries count:", cur.fetchone()[0])

cur.execute("SELECT COUNT(*) FROM sessions")
print("sessions count:", cur.fetchone()[0])

cur.execute("PRAGMA table_info(sessions)")
print("sessions columns:", [r[1] for r in cur.fetchall()])

cols = [r[1] for r in con.execute("PRAGMA table_info(transcript_entries)").fetchall()]
rows = cur.execute("SELECT * FROM transcript_entries").fetchall()
print("rows fetched:", len(rows))
hits = []
for row in rows:
    record = dict(zip(cols, row))
    blob = json.dumps(record, ensure_ascii=False, default=str)
    if TARGET in blob or '"plan"' in blob.lower() or "planRevision" in blob:
        hits.append(record)
print("entries mentioning target/plan:", len(hits))
for i, rec in enumerate(hits[:6]):
    print("=" * 40, f"hit {i}")
    for k, v in rec.items():
        if isinstance(v, str) and len(v) > 600:
            print(f"  {k}: <{len(v)} chars>")
            print("   ", v[:600].replace("\n", "\\n"))
        else:
            print(f"  {k}: {v}")

# also list sessions
cur.execute("SELECT session_id, title FROM sessions LIMIT 10")
print("-" * 40)
for r in cur.fetchall():
    print("session:", (r[0] or "")[:40], "|", (r[1] or "")[:40])
con.close()
