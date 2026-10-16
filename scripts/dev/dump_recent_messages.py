"""Inspect the most recent messages in the Rust gateway message store."""
import json
import os
import sqlite3

con = sqlite3.connect(os.path.expandvars(r"%USERPROFILE%\.marguerite\sessions.db"))
cur = con.cursor()
cur.execute("SELECT id, session_id, role, content, thinking, tool_name, tool_input, tool_output, timestamp FROM session_messages ORDER BY id DESC LIMIT 12")
for r in cur.fetchall():
    mid, sid, role, content, thinking, tool_name, tool_input, tool_output, ts = r
    print("=" * 60)
    print(f"id={mid} session={sid[:30]} role={role} tool={tool_name} ts={ts}")
    for label, val in (("content", content), ("tool_input", tool_input), ("tool_output", tool_output)):
        if isinstance(val, str) and val:
            snippet = val[:300].replace("\n", " ")
            print(f"  {label}: {snippet}")
con.close()
