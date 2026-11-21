#!/bin/zsh
# usage: start-api.sh <port> <db> [app]   (app => configure GitHub App env)
PORT=$1; DB=$2; APP=$3
SP=/private/tmp/claude-501/-Users-andreangelantoni-Projects-gridops--claude-worktrees-ui-145-platform-neutral-cf5832/fff39e50-a1ac-48a4-a4c2-15939665982d/scratchpad
WT=/Users/andreangelantoni/Projects/gridops/.claude/worktrees/ui-145-platform-neutral-cf5832
export GRIDOPS_BASE_URL=http://127.0.0.1:$PORT GRIDOPS_API_BIND=127.0.0.1:$PORT GRIDOPS_DATABASE_PATH=$DB
export GRIDOPS_LOG_DIRECTORY=$SP/data/logs GRIDOPS_WEB_ROOT=${WEB_ROOT:-$WT/dist}
export GRIDOPS_SESSION_SECRET=audit-session-secret-for-s-walk GRIDOPS_ENCRYPTION_KEY=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=
export GRIDOPS_FLEET_ALLOW_LOOPBACK_HTTP=true GITHUB_CLIENT_ID=audit-client GITHUB_CLIENT_SECRET=audit-secret GITHUB_WEBHOOK_SECRET=audit-webhook GRIDOPS_WEBHOOK_ACTIVE=false GRIDOPS_MANAGER_URL=http://127.0.0.1:9
if [ "$APP" = "app" ]; then
  export GITHUB_APP_ID=1 GITHUB_APP_SLUG=gridops-audit GITHUB_APP_PRIVATE_KEY="$(cat $SP/data/app.pem)"
fi
exec $WT/target/debug/gridops-api
