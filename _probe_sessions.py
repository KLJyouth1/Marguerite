import sqlite3, os
db = r'C:\Users\Administrator\.marguerite\sessions.db'
c = sqlite3.connect(db)
print("=== ~/.marguerite/sessions.db ===")
print([[r[0] for r in c.execute("SELECT name FROM sqlite_master WHERE type='table'")]])
print("session_id | project_path | updated_at")
for r in c.execute("SELECT session_id, COALESCE(project_path,'<NULL>'), updated_at FROM sessions ORDER BY updated_at DESC LIMIT 40"):
    print(r)