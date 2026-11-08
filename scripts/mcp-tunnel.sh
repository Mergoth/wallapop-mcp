#!/usr/bin/env bash
# Open a persistent SSH tunnel to the NAS MCP server.
# Run once; the tunnel stays open in the background.
# Claude Desktop then connects to http://localhost:8080/sse
#
# Usage: ./scripts/mcp-tunnel.sh [start|stop|status]

NAS_HOST="192.168.1.99"
NAS_PORT="2300"
NAS_USER="vlad"
LOCAL_PORT="8080"
REMOTE_PORT="8080"
PID_FILE="/tmp/wallapop-mcp-tunnel.pid"

start() {
  if [[ -f "$PID_FILE" ]] && kill -0 "$(cat "$PID_FILE")" 2>/dev/null; then
    echo "Tunnel already running (PID $(cat "$PID_FILE"))"
    return 0
  fi
  ssh -f -N -o ExitOnForwardFailure=yes -o ServerAliveInterval=30 \
      -p "$NAS_PORT" \
      -L "${LOCAL_PORT}:localhost:${REMOTE_PORT}" \
      "${NAS_USER}@${NAS_HOST}"
  # Find the PID of the background ssh process
  sleep 0.5
  pgrep -n -f "ssh.*${LOCAL_PORT}:localhost:${REMOTE_PORT}" > "$PID_FILE"
  echo "Tunnel started (PID $(cat "$PID_FILE")). Connect agents to http://localhost:${LOCAL_PORT}/sse"
}

stop() {
  if [[ -f "$PID_FILE" ]]; then
    kill "$(cat "$PID_FILE")" 2>/dev/null && rm -f "$PID_FILE" && echo "Tunnel stopped."
  else
    echo "No tunnel PID file found."
  fi
}

status() {
  if [[ -f "$PID_FILE" ]] && kill -0 "$(cat "$PID_FILE")" 2>/dev/null; then
    echo "Tunnel is running (PID $(cat "$PID_FILE")) → http://localhost:${LOCAL_PORT}/sse"
  else
    echo "Tunnel is not running."
  fi
}

case "${1:-start}" in
  start)  start  ;;
  stop)   stop   ;;
  status) status ;;
  *) echo "Usage: $0 [start|stop|status]"; exit 1 ;;
esac
