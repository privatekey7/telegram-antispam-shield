#!/bin/sh
# Roll the bot to a given image tag and roll back if it does not come up.
#
# Usage: deploy.sh sha-<40-char commit>
# Lives next to compose.yaml and .env (docs/auto-deploy.md). CI calls it
# through an SSH forced command, so the tag arrives in SSH_ORIGINAL_COMMAND;
# run by hand, pass it as the first argument.
set -eu

# .env holds BOT_TOKEN: every file written below stays owner-only.
umask 077

cd "$(dirname "$0")"

TAG="${1:-${SSH_ORIGINAL_COMMAND:-}}"
STARTUP_WAIT_SECS="${STARTUP_WAIT_SECS:-15}"

# The tag is untrusted input from the SSH session: accept only the exact shape.
# Check the whole value with shell patterns, not grep: grep matches line by
# line, so "sha-<sha>\nIMAGE=..." would pass and inject lines into .env.
valid_tag() {
    case "$1" in
        sha-*) ;;
        *) return 1 ;;
    esac
    hex="${1#sha-}"
    [ "${#hex}" -eq 40 ] || return 1
    # Spelled out rather than a-f, so the locale cannot widen the range.
    case "$hex" in
        *[!0123456789abcdef]*) return 1 ;;
    esac
}

if ! valid_tag "$TAG"; then
    echo "usage: deploy.sh sha-<40-char commit sha>" >&2
    exit 2
fi

is_running() {
    [ -n "$(docker compose ps --status running --quiet bot)" ]
}

up() {
    docker compose pull --quiet bot &&
        docker compose up --detach --remove-orphans
}

# Keep the bot settings and other compose variables (e.g. IMAGE on forks);
# replace only IMAGE_TAG.
touch .env
cp .env .env.previous
grep -v '^IMAGE_TAG=' .env > .env.next || true
printf 'IMAGE_TAG=%s\n' "$TAG" >> .env.next
mv .env.next .env

echo "deploying $TAG"
# A failed pull or start goes to the rollback below instead of ending the
# script under set -e with .env already on the new tag.
if up && sleep "$STARTUP_WAIT_SECS" && is_running; then
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
