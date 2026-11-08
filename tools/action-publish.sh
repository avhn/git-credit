#!/usr/bin/env bash
# Publish only recognized generated assets, with an orphan root and explicit lease.
set -euo pipefail

branch="${GC_PUBLISH_BRANCH:?}"
git check-ref-format "refs/heads/$branch"
if [ "$branch" = "${GC_DEFAULT_BRANCH:-main}" ] || [ "refs/heads/$branch" = "${GITHUB_REF:-}" ]; then
  echo 'Refusing to replace the source/default branch with badge assets' >&2
  exit 1
fi
if [ "$(git rev-parse --is-shallow-repository)" != false ]; then
  echo 'git-credit requires fetch-depth: 0; run git fetch --unshallow.' >&2
  exit 1
fi
if [[ ! "${GITHUB_REPOSITORY:?}" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]]; then
  echo 'Invalid GitHub repository name' >&2; exit 1
fi
server="${GITHUB_SERVER_URL:-https://github.com}"
if [[ ! "$server" =~ ^https://[A-Za-z0-9.-]+(:[0-9]+)?$ ]]; then
  echo 'Expected an HTTPS GitHub server URL' >&2; exit 1
fi
remote="$server/$GITHUB_REPOSITORY.git"
sha="$(git rev-parse HEAD)"
if [ -n "${GITHUB_SHA:-}" ] && [ "$sha" != "$GITHUB_SHA" ]; then
  echo 'Checked-out HEAD differs from the triggering SHA; refusing publication' >&2; exit 1
fi
short="${sha:0:12}"
# Keep helper scripts separate from the generated-only repository and archive data.
helpers="$(mktemp -d "${RUNNER_TEMP:?}/git-credit-publish-helper.XXXXXX")"
stage="$(mktemp -d "$RUNNER_TEMP/git-credit-publish.XXXXXX")"
trap 'rm -rf -- "$stage" "$helpers"' EXIT
cat > "$helpers/askpass.sh" <<'ASKPASS'
#!/usr/bin/env bash
case "$1" in
  *Username*) printf '%s\n' x-access-token ;;
  *Password*) printf '%s\n' "${GC_TOKEN:?}" ;;
  *) exit 1 ;;
esac
ASKPASS
chmod 700 "$helpers/askpass.sh"
export GIT_ASKPASS="$helpers/askpass.sh" GIT_TERMINAL_PROMPT=0
# Disable inherited credential helpers and hooks for every publishing Git command.
export GIT_CONFIG_COUNT=3
export GIT_CONFIG_KEY_0=credential.helper GIT_CONFIG_VALUE_0=
export GIT_CONFIG_KEY_1=core.hooksPath GIT_CONFIG_VALUE_1=/dev/null
export GIT_CONFIG_KEY_2=commit.gpgsign GIT_CONFIG_VALUE_2=false
python_cmd=python3
if [ "${RUNNER_OS:-}" = Windows ]; then python_cmd=python; fi
"$python_cmd" -I "$GITHUB_ACTION_PATH/tools/stage-badges.py" "${GC_OUT_DIR:?}" "$stage"
git -C "$stage" init -q -b generated
# Fresh repository: commit has no parent and contains no checkout source files.
git -C "$stage" add -f -- .
git -C "$stage" -c user.name='github-actions[bot]' -c user.email='41898282+github-actions[bot]@users.noreply.github.com' commit -q -m "chore(badges): update for $short"
ref="refs/heads/$branch"
if git ls-remote --exit-code --refs "$remote" "$ref" > /dev/null; then
  git -C "$stage" fetch -q --no-tags --depth=1 "$remote" "$ref"
  previous="$(git -C "$stage" rev-parse FETCH_HEAD)"
else
  status=$?
  if [ "$status" != 2 ]; then echo 'Cannot inspect badge branch' >&2; exit "$status"; fi
  previous=
fi
# Check freshness immediately before pushing, not just before the analysis/build.
if [ -n "${GC_SOURCE_BRANCH:-}" ]; then
  git check-ref-format "refs/heads/$GC_SOURCE_BRANCH"
  current="$(git ls-remote --exit-code --refs "$remote" "refs/heads/$GC_SOURCE_BRANCH")"
  current="${current%%$'\t'*}"
  if [ "$current" != "$sha" ]; then
    echo "Skipping stale badge run: triggering SHA is no longer $GC_SOURCE_BRANCH HEAD" >&2
    exit 0
  fi
fi
# Empty expected value is a creation-only lease: a racing first publisher cannot
# overwrite a newly created branch. Existing branches lease the fetched object ID.
git -C "$stage" push --force-with-lease="$ref:$previous" "$remote" "HEAD:$ref"
