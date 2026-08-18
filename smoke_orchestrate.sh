#!/usr/bin/env bash
# Marguerite gateway smoke-test orchestrator (ISOLATED mode)
# Runs the freshly-built gateway on an isolated port + isolated HOME so it uses
# fresh, writable SQLite DBs and never conflicts with the live gateway (which
# holds ~/.marguerite/sessions.db read-only for other writers). Non-disruptive.
set -u
BIN="D:/tp_build_fresh1/release/marguerite-core.exe"
ROOT="D:/Downloads/Marguerite/marguerite"
LOG="$ROOT/smoke_gw_run.log"
RESULT="$ROOT/smoke_result_iso.json"
SMOKE_PORT=18799
SMOKE_HOME="D:/tp_smoke_home"
WS_URL="ws://127.0.0.1:$SMOKE_PORT/ws"

now(){ date +"%Y-%m-%dT%H:%M:%S"; }
log(){ echo "[$(now)] $*"; }

# Load API key/model from project .env (so the engine can call the LLM if reachable)
API_KEY=$(grep -E '^MARGUERITE_API_KEY=' "$ROOT/.env" 2>/dev/null | head -1 | cut -d= -f2-)
API_MODEL=$(grep -E '^MARGUERITE_MODEL=' "$ROOT/.env" 2>/dev/null | head -1 | cut -d= -f2-)
API_KEY="${API_KEY:-sk-bfd59e486be843889b6ef3503f6429f4}"
API_MODEL="${API_MODEL:-deepseek-v4-pro}"

log "orchestrator start; waiting for fresh binary: $BIN"

# ---- wait for the freshly-built binary ----
READY=0
for i in $(seq 1 900); do
  if [ -s "$BIN" ]; then
    s1=$(stat -c%s "$BIN" 2>/dev/null || echo 0); sleep 4
    s2=$(stat -c%s "$BIN" 2>/dev/null || echo 0)
    if [ "$s1" = "$s2" ] && [ "$s2" -gt 0 ]; then log "binary ready (size=$s2) after ~$((i*5))s"; READY=1; break; fi
  fi
  sleep 5
done
if [ "$READY" -ne 1 ]; then
  log "ERROR: binary never appeared -> build likely failed. tail build log:"
  tail -15 "$ROOT/build_clean_2026-08-17.log" 2>/dev/null
  exit 2
fi

# ---- free only the SMOKE_PORT (never touch live 18790) ----
PID=$(netstat -ano 2>/dev/null | grep -i ":$SMOKE_PORT" | grep -i "LISTENING" | awk '{print $5}' | head -1)
if [ -n "$PID" ]; then taskkill //F //PID "$PID" 2>/dev/null; sleep 2; fi

# ---- isolated home (fresh, writable DBs) ----
rm -rf "$SMOKE_HOME" 2>/dev/null
mkdir -p "$SMOKE_HOME"

log "launching gateway (isolated home=$SMOKE_HOME, port=$SMOKE_PORT)"
cd "$ROOT" || exit 3
USERPROFILE="$SMOKE_HOME" HOME="$SMOKE_HOME" \
  MARGUERITE_API_KEY="$API_KEY" MARGUERITE_MODEL="$API_MODEL" \
  nohup "$BIN" gateway --port "$SMOKE_PORT" > "$LOG" 2>&1 &
GW_PID=$!
log "gateway pid=$GW_PID"

# ---- wait for listening socket ----
UP=0
for i in $(seq 1 40); do
  if netstat -ano 2>/dev/null | grep -i ":$SMOKE_PORT" | grep -i "LISTENING" >/dev/null; then
    log "gateway listening on $SMOKE_PORT"; UP=1; break
  fi
  sleep 1
done
if [ "$UP" -ne 1 ]; then
  log "ERROR: gateway did not come up. tail run log:"
  tail -25 "$LOG" 2>/dev/null
  taskkill //F //PID "$GW_PID" 2>/dev/null
  exit 4
fi

# ---- run smoke client ----
log "running smoke client ($ROOT/smoke_gateway.cjs)"
export MARG_WS_URL="$WS_URL"
export MARG_RESULT="$RESULT"
node "$ROOT/smoke_gateway.cjs"
RC=$?
log "smoke client rc=$RC"

# ---- report ----
log "===== SMOKE RESULT ====="
cat "$RESULT" 2>/dev/null | head -90

# ---- teardown (only our gateway) ----
log "stopping gateway (pid=$GW_PID)"
taskkill //F //PID "$GW_PID" 2>/dev/null
log "done"
