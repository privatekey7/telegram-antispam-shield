#!/bin/sh
# Roll the bot to a given image tag and roll back if it does not come up.
#
# Usage: deploy.sh sha-<40-char commit>
# CI calls it through an SSH forced command, so the tag arrives in
# SSH_ORIGINAL_COMMAND; run by hand, pass it as the first argument.
set -eu

cd "$(dirname "$0")"

TAG="${1:-${SSH_ORIGINAL_COMMAND:-}}"
STARTUP_WAIT_SECS="${STARTUP_WAIT_SECS:-15}"

# The tag is untrusted input from the SSH session: accept only the exact shape.
if ! printf '%s' "$TAG" | grep -Eq '^sha-[0-9a-f]{40}$'; then
    echo "usage: deploy.sh sha-<40-char commit sha>" >&2
    exit 2
fi

is_running() {
    [ -n "$(docker compose ps --status running --quiet bot)" ]
}

up() {
    docker compose pull --quiet bot
    docker compose up --detach --remove-orphans
}

# Keep other compose variables (e.g. IMAGE on forks); replace only IMAGE_TAG.
touch .env
cp .env .env.previous
grep -v '^IMAGE_TAG=' .env > .env.next || true
printf 'IMAGE_TAG=%s\n' "$TAG" >> .env.next
mv .env.next .env

echo "deploying $TAG"
up
sleep "$STARTUP_WAIT_SECS"

if is_running; then
    docker image prune --force > /dev/null
    echo "deployed $TAG"
    exit 0
fi

echo "bot did not stay up; last logs:" >&2
docker compose logs --tail 50 bot >&2 || true
if grep -q '^IMAGE_TAG=' .env.previous; then
    echo "rolling back to $(grep '^IMAGE_TAG=' .env.previous)" >&2
    cp .env.previous .env
    up
fi
exit 1
