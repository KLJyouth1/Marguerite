import sqlite3, os

targets = [
    (r"$LOCALAPPDATA\Marguerite\sessions.db", "appdata-root"),
    (r"$HOME\.marguerite\sessions.db", "marguerite-home"),
    (r"$LOCALAPPDATA\Marguerite\pyengine-home\default\state\sessions.db", "pyengine-default"),
]

target = "rev-573410db-428b-40d5-b1e9-aaee0a7881f3"

for path, label in targets:
    p = path.replace("$LOCALAPPDATA", os.environ["LOCALAPPDATA"]).replace("$HOME", os.environ["USERPROFILE"])
    print("=" * 20, label, p)
    try:
        con = sqlite3.connect(p, uri=True)
        cur = con.cursor()
        cur.execute("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        tables = [r[0] for r in cur.fetchall()]
        print("tables:", tables)
        if "plan_revisions" in tables:
            cur.execute("SELECT COUNT(*) FROM plan_revisions")
            print("plan_revisions count:", cur.fetchone()[0])
            cur.execute("SELECT revision_id, plan_id, title, source_session_key FROM plan_revisions LIMIT 5")
            for r in cur.fetchall():
                print("  rev:", r[0][:28], "| plan:", (r[1] or "")[:18], "| src:", (r[3] or "")[-22:], "| title:", (r[2] or "")[:30])
            cur.execute("SELECT revision_id FROM plan_revisions WHERE revision_id = ?", (target,))
            print("target found:", cur.fetchone() is not None)
        if "sessions" in tables:
            cur.execute("SELECT COUNT(*) FROM sessions")
            print("sessions count:", cur.fetchone()[0])
        con.close()
    except Exception as exc:
        print("ERR:", exc)
