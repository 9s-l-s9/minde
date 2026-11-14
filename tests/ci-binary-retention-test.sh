#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Exercise the real retention planner/executor against a stateful fake API.
set -eu
cd "$(dirname "$0")/.."
root=$PWD
scratch=$(mktemp -d "${TMPDIR:-/tmp}/minde-retention-test.XXXXXX")
trap 'rm -rf "$scratch"' EXIT
trap 'exit 1' HUP INT TERM
mkdir "$scratch/bin"
export RETENTION_TEST_DIR="$scratch"
export GITHUB_ACTIONS=true GITHUB_REPOSITORY=9s-l-s9/minde GITHUB_REF=refs/heads/main
export GITHUB_WORKFLOW_REF=9s-l-s9/minde/.github/workflows/guix-binary.yml@refs/heads/main
GITHUB_SHA=$(printf '%040d' 18)
export GITHUB_SHA
unset RETENTION_FAIL_LIST RETENTION_FAIL_DELETE RETENTION_CHANGE
cat >"$scratch/bin/gh" <<'EOF'
#!/bin/sh
set -eu
cd "$RETENTION_TEST_DIR"
[ "$1" = api ] || exit 90
shift
method=GET endpoint= paginate=false
while [ "$#" -gt 0 ]; do
    case $1 in
        --hostname) [ "$2" = github.com ] || exit 91; shift 2 ;;
        -H) shift 2 ;;
        --method) method=$2; shift 2 ;;
        --paginate) paginate=true; shift ;;
        repos/9s-l-s9/minde/*) endpoint=$1; shift ;;
        *) echo "unexpected gh argument: $1" >&2; exit 92 ;;
    esac
done
if [ "$method" = DELETE ]; then
    printf '%s\n' "$endpoint" >>mutations
    case $endpoint in
        */releases/*)
            id=${endpoint##*/}
            [ "${RETENTION_FAIL_DELETE:-}" != release ] || exit 73
            jq --argjson id "$id" '.releases |= map(select(.id != $id))' state.json >next.json ;;
        */git/refs/tags/*)
            [ "${RETENTION_FAIL_DELETE:-}" != tag ] || exit 74
            tag=${endpoint##*/}
            jq --arg tag "refs/tags/$tag" '.refs |= map(select(.ref != $tag))' state.json >next.json ;;
        *) exit 93 ;;
    esac
    mv next.json state.json
    exit 0
fi
[ "$method" = GET ] && "$paginate" || exit 94
case $endpoint in
    */releases\?per_page=100)
        field=releases
        count=$(cat reads); count=$((count + 1)); printf '%s\n' "$count" >reads
        if [ "$count" -eq 2 ] && [ "${RETENTION_CHANGE:-}" = race ]; then
            jq '
              .refs |= map(if .ref == ("refs/tags/guix-" + ("0" * 39) + "1")
                           then .object.sha = ("f" * 40) else . end)
              | .runs |= map(if .head_sha == (("0" * 39) + "3") then .status = "in_progress" else . end)
              | .releases += [{id: 113, tag_name: ("guix-" + ("0" * 38) + "13"),
                              draft: true, prerelease: true}]
            ' state.json >next.json
            mv next.json state.json
        fi ;;
    */git/matching-refs/tags/guix-) field=refs ;;
    */actions/workflows/guix-binary.yml/runs\?per_page=100) field=runs ;;
    *) echo "unexpected endpoint: $endpoint" >&2; exit 95 ;;
esac
[ "${RETENTION_FAIL_LIST:-}" != "$field" ] || exit 71
if [ "${RETENTION_FAIL_LIST:-}" = malformed ]; then printf '{"message":"bad page"}\n'; exit 0; fi
# Multiple JSON documents model gh --paginate, including empty final pages.
if [ "$field" = runs ]; then
    jq '.runs[:10] | {workflow_runs: .}' state.json
    [ "${RETENTION_FAIL_LIST:-}" != later-page ] || exit 72
    jq '.runs[10:] | {workflow_runs: .}' state.json
else
    jq --arg field "$field" '.[$field][:5], .[$field][5:]' state.json
fi
EOF
chmod +x "$scratch/bin/gh"
PATH="$scratch/bin:$PATH"
export PATH
fail() { echo "FAIL - $*" >&2; exit 1; }
reset() {
    : >"$scratch/mutations"
    printf '0\n' >"$scratch/reads"
    jq -n '
      def sha($n): ("0" * 40 + ($n | tostring))[-40:];
      def release($n): {id: $n, tag_name: ("guix-" + sha($n)), draft: false, prerelease: true,
         immutable: false, author: {login: "github-actions[bot]"},
         published_at: ("2026-11-" + ("0" + ($n | tostring))[-2:] + "T00:00:00Z")};
      {releases: (([range(1; 13) | release(.)]
        | map(if .id == 9 then .draft = true | .published_at = null
              elif .id == 10 then .prerelease = false
              elif .id == 11 then .immutable = true
              elif .id == 12 then .author.login = "maintainer" else . end))
          + [{id: 91, tag_name: "v1.0.0", draft: false, prerelease: false}]),
       refs: (([range(1; 19) | {ref: ("refs/tags/guix-" + sha(.)), object: {sha: sha(.), type: "commit"}}]
        | map(if .ref == ("refs/tags/guix-" + sha(16)) then .object.sha = sha(99)
              elif .ref == ("refs/tags/guix-" + sha(17)) then .object.type = "tag" else . end))
          + [{ref: "refs/tags/guix-main", object: {sha: sha(1), type: "commit"}},
             {ref: "refs/tags/v1.0.0", object: {sha: sha(1), type: "commit"}}]),
       runs: [range(1; 19) | select(. != 15) | . as $n |
         {head_sha: sha($n), head_branch: "main", head_repository: {full_name: "9s-l-s9/minde"},
          status: (if $n == 2 then "waiting" elif $n == 14 then "queued" else "completed" end)}]}
    ' >"$scratch/state.json"
}
run() { sh "$root/scripts/ci-binary-retention" "$@" >"$scratch/result" 2>"$scratch/error"; }
ok() { run "$@" || { cat "$scratch/error" >&2; fail "retention $*"; }; }
assert_state() { jq -e "$1" "$scratch/state.json" >/dev/null || fail "$2"; }
unchanged() { [ ! -s "$scratch/mutations" ] || fail "$1 caused a remote mutation"; }

reset
ok --plan
jq -e '[.[] | [.kind, (.sha[-2:] | tonumber)]] == [["release", 3], ["release", 1], ["tag", 13]]' \
    "$scratch/result" >/dev/null || fail 'incorrect paginated plan'
unchanged plan
echo 'ok - paginated plan keeps five newest builds and protects active and unrelated references'

ok --apply
assert_state '[.releases[].id] | sort == [2,4,5,6,7,8,9,10,11,12,91]' 'wrong releases removed'
assert_state '[.refs[].ref] | length == 17' 'wrong tag removal count'
for n in 1 3 13; do
    tag=$(printf 'refs/tags/guix-%040d' "$n")
    jq -e --arg tag "$tag" 'all(.refs[]; .ref != $tag)' "$scratch/state.json" >/dev/null || fail "tag $n survived"
done
: >"$scratch/mutations"
ok --apply
unchanged 'repeat cleanup'
echo 'ok - apply removes only selected releases/tags and is idempotent'

reset
export RETENTION_CHANGE=race
ok --apply
unchanged 'retargeted tag, restarted run and newly published orphan'
unset RETENTION_CHANGE
echo 'ok - revalidation protects changed tag targets, reruns and newly attached releases'

for problem in releases refs runs later-page malformed; do
    reset
    export RETENTION_FAIL_LIST=$problem
    if run --apply; then fail "accepted $problem API failure"; fi
    unchanged "$problem API failure"
done
unset RETENTION_FAIL_LIST
for malformed in 'del(.runs[0].status)' '.runs[0].status = ""' '.runs[0].head_sha = "invalid"' \
    '.releases += [.releases[0]]' '.releases += [(.releases[0] | .id = 100)]' \
    '.releases += [(.releases[0] | .tag_name = "guix-duplicate")]' '.refs += [.refs[0]]'; do
    reset
    jq "$malformed" "$scratch/state.json" >"$scratch/next.json"
    mv "$scratch/next.json" "$scratch/state.json"
    if run --apply; then fail "accepted malformed data: $malformed"; fi
    unchanged "malformed data: $malformed"
done
echo 'ok - failed, partial and malformed API snapshots cause no deletion'

reset
export RETENTION_FAIL_DELETE=release
if run --apply; then fail 'ignored release deletion failure'; fi
assert_state 'any(.releases[]; .id == 3)' 'failed release removed'
[ "$(wc -l <"$scratch/mutations")" -eq 1 ] || fail 'continued after release deletion failure'
unset RETENTION_FAIL_DELETE
reset
export RETENTION_FAIL_DELETE=tag
if run --apply; then fail 'ignored tag deletion failure'; fi
assert_state 'all(.releases[]; .id != 3)' 'release deletion did not precede tag deletion'
assert_state 'any(.refs[]; .ref == ("refs/tags/guix-" + ("0" * 39) + "3"))' 'failed tag deletion changed ref'
unset RETENTION_FAIL_DELETE
ok --apply
assert_state 'all(.refs[]; .ref != ("refs/tags/guix-" + ("0" * 39) + "3"))' 'retry failed to clean orphan'
echo 'ok - deletion failures stop the batch and a retry repairs orphaned tags'

reset
GITHUB_SHA=$(printf '%040d' 1)
export GITHUB_SHA
ok --apply
assert_state 'any(.releases[]; .id == 1)' 'current old build was removed'
for context in fork branch workflow sha local; do
    reset
    if (
        case $context in
            fork) GITHUB_REPOSITORY=someone/fork ;;
            branch) GITHUB_REF=refs/heads/feature ;;
            workflow) GITHUB_WORKFLOW_REF=9s-l-s9/minde/.github/workflows/other.yml@refs/heads/main ;;
            sha) GITHUB_SHA= ;;
            local) GITHUB_ACTIONS=false ;;
        esac
        run --apply
    ); then fail "accepted $context application context"; fi
    unchanged "$context application context"
done
echo 'ok - current commit is retained and only the canonical workflow can apply retention'
