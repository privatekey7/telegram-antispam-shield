#!/bin/sh
# Tests for deploy.sh: tag validation, the .env rewrite and the rollback.
#
# Usage: sh deploy/test-deploy.sh
# Runs deploy.sh under every available shell (sh, dash, bash) in a scratch
# directory, with a stub `docker` first in PATH, so nothing touches Docker.
set -eu

here="$(cd "$(dirname "$0")" && pwd)"
sha="0123456789abcdef0123456789abcdef01234567"
digest="89abcdef0123456789abcdef0123456789abcdef0123456789abcdef01234567"
good="deploy-$sha@sha256:$digest"
failures=0

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# Stub docker: logs every call; `compose ps` reports the bot as running;
# `compose pull` fails for the tag in STUB_BAD_TAG, read from ./.env.
mkdir "$work/bin"
cat > "$work/bin/docker" <<'EOF'
#!/bin/sh
echo "docker $*" >> docker.log
case "$*" in
    "compose pull"*)
        if [ -n "${STUB_BAD_TAG:-}" ] && grep -qx "IMAGE_TAG=$STUB_BAD_TAG" .env; then
            exit 1
        fi
        ;;
    "compose ps"*) echo 0123abcd ;;
esac
EOF
chmod 755 "$work/bin/docker"

fail() {
    echo "FAIL [$shell] $1" >&2
    failures=$((failures + 1))
}

# Fresh deploy directory with a copy of deploy.sh and an existing .env.
setup() {
    dir="$work/run"
    rm -rf "$dir"
    mkdir "$dir"
    cp "$here/deploy.sh" "$dir/"
    printf 'BOT_TOKEN=x\nIMAGE_TAG=sha-%040d\n' 0 > "$dir/.env"
    cp "$dir/.env" "$dir/.env.orig"
}

run() {
    (cd "$dir" && PATH="$work/bin:$PATH" STARTUP_WAIT_SECS=0 \
        "$shell" ./deploy.sh "$@") > "$dir/out.log" 2>&1
}

expect_rejected() {
    name="$1"
    setup
    status=0
    run "$2" || status=$?
    [ "$status" -eq 2 ] || fail "$name: exit $status, want 2"
    cmp -s "$dir/.env" "$dir/.env.orig" || fail "$name: .env changed"
    [ ! -e "$dir/docker.log" ] || fail "$name: docker was called"
}

nl='
'
cr="$(printf '\r')"

for shell in sh dash bash; do
    command -v "$shell" > /dev/null 2>&1 || continue

    expect_rejected "newline injection" "$good${nl}IMAGE=evil.example/x"
    expect_rejected "newline first" "IMAGE=evil.example/x${nl}$good"
    expect_rejected "trailing newline" "$good$nl"
    expect_rejected "carriage return" "$good$cr"
    expect_rejected "uppercase hex" "deploy-$(printf '%s' "$sha" | tr a-f A-F)@sha256:$digest"
    expect_rejected "uppercase digest" "deploy-$sha@sha256:$(printf '%s' "$digest" | tr a-f A-F)"
    expect_rejected "uppercase prefix" "DEPLOY-$sha@sha256:$digest"
    expect_rejected "short commit" "deploy-${sha%?}@sha256:$digest"
    expect_rejected "short digest" "deploy-$sha@sha256:${digest%?}"
    expect_rejected "long" "${good}0"
    expect_rejected "no digest" "deploy-$sha"
    expect_rejected "old sha- tag" "sha-$sha"
    expect_rejected "two digests" "$good@sha256:$digest"
    expect_rejected "other algorithm" "deploy-$sha@sha512:$digest"
    expect_rejected "trailing space" "$good "
    expect_rejected "leading space" " $good"
    expect_rejected "command substitution" "deploy-\$(id)${sha#??????}@sha256:$digest"
    expect_rejected "no prefix" "$sha@sha256:$digest"
    expect_rejected "empty" ""

    # The forced command passes the tag in SSH_ORIGINAL_COMMAND.
    setup
    status=0
    (cd "$dir" && PATH="$work/bin:$PATH" STARTUP_WAIT_SECS=0 \
        SSH_ORIGINAL_COMMAND="$good${nl}IMAGE=evil.example/x" \
        "$shell" ./deploy.sh) > "$dir/out.log" 2>&1 || status=$?
    [ "$status" -eq 2 ] || fail "SSH_ORIGINAL_COMMAND injection: exit $status, want 2"
    cmp -s "$dir/.env" "$dir/.env.orig" || fail "SSH_ORIGINAL_COMMAND injection: .env changed"

    # A good tag replaces IMAGE_TAG and keeps the other settings.
    setup
    status=0
    run "$good" || status=$?
    [ "$status" -eq 0 ] || fail "good tag: exit $status, want 0"
    [ "$(cat "$dir/.env")" = "BOT_TOKEN=x${nl}IMAGE_TAG=$good" ] ||
        fail "good tag: unexpected .env: $(cat "$dir/.env")"
    cmp -s "$dir/.env.previous" "$dir/.env.orig" || fail "good tag: .env.previous is not the old .env"

    # A failed pull rolls back instead of leaving .env on the new tag.
    setup
    status=0
    (export STUB_BAD_TAG="$good" && run "$good") || status=$?
    [ "$status" -eq 1 ] || fail "failed pull: exit $status, want 1"
    cmp -s "$dir/.env" "$dir/.env.orig" || fail "failed pull: .env not rolled back"
done

if [ "$failures" -ne 0 ]; then
    echo "$failures failure(s)" >&2
    exit 1
fi
echo "deploy.sh: all tests passed"
