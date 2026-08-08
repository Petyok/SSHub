#!/usr/bin/env bash
# Validate a branch name against the naming rules in AGENTS.md.
#
#   check-branch-name.sh <branch> [commit-type ...]
#
# With no commit types, only the *shape* is checked: `<prefix>/<slug>` with a
# known prefix. That is all a local hook can honestly know — the commit that
# justifies the prefix may not be written yet.
#
# With commit types (the conventional-commit type of every commit on the
# branch), the *substance* is checked too: at least one commit must justify the
# prefix. A `fix/` branch needs a `fix:` commit; supporting `test:` / `docs:`
# commits alongside it are fine. This is what caught `feature/` being used for
# a pure bug fix, which is the whole reason this script exists.
#
# `--self-test` runs the table of cases below and is what CI runs to check the
# checker.
set -euo pipefail

# Branches git or a workflow owns; not ours to name.
EXEMPT='^(main|master|development|HEAD|openwiki/update)$'

# prefix:type[,type...] — which commit types justify which prefix.
#
# `poc` accepts everything on purpose: an epic's exploratory child branch has no
# predictable shape, so only its name is worth checking (AGENTS.md § Workflow).
JUSTIFIES='
feature:feat
fix:fix
docs:docs
chore:chore,ci,build,refactor,perf,style,test
poc:feat,fix,docs,chore,ci,build,refactor,perf,style,test
'

prefixes() { echo "$JUSTIFIES" | sed '/^$/d' | cut -d: -f1; }

types_for() {
    echo "$JUSTIFIES" | sed '/^$/d' | while IFS=: read -r p t; do
        if [ "$p" = "$1" ]; then echo "$t" | tr ',' ' '; fi
    done
}

check() {
    branch=$1
    shift
    commit_types="$*"

    if echo "$branch" | grep -qE "$EXEMPT"; then return 0; fi

    prefix=${branch%%/*}
    slug=${branch#*/}
    if [ "$prefix" = "$branch" ] || [ -z "$slug" ]; then
        echo "branch '$branch': expected <prefix>/<slug>, e.g. fix/quoted-aliases" >&2
        return 1
    fi
    if ! prefixes | grep -qx "$prefix"; then
        echo "branch '$branch': unknown prefix '$prefix' (allowed: $(prefixes | tr '\n' ' '))" >&2
        return 1
    fi

    # Shape-only mode: nothing more can be said without the branch's commits.
    if [ -z "$commit_types" ]; then return 0; fi

    allowed=$(types_for "$prefix")
    # `case` rather than a nested `[ ] &&` loop: under `set -e` a test that
    # fails as the last command of a loop body aborts the whole script, which
    # silently turned every rejection into a message-less exit 1.
    for t in $commit_types; do
        case " $allowed " in
        *" $t "*) return 0 ;;
        esac
    done
    echo "branch '$branch': no commit justifies the '$prefix/' prefix." >&2
    echo "  commit types on the branch: $commit_types" >&2
    echo "  '$prefix/' wants one of:     $allowed" >&2
    echo "  Rename the branch to match what the work actually is (AGENTS.md § Branch naming)." >&2
    return 1
}

self_test() {
    fails=0
    # branch | commit types | expected exit
    while IFS='|' read -r branch types want; do
        [ -z "${branch// /}" ] && continue
        got=0
        check "$branch" $types >/dev/null 2>&1 || got=1
        if [ "$got" != "$want" ]; then
            echo "FAIL: check('$branch', '$types') = $got, want $want" >&2
            fails=$((fails + 1))
        fi
    done <<'CASES'
feature/profiles|feat|0
feature/profiles|feat test docs|0
feature/profiles|fix|1
feature/profiles||0
fix/quoted-aliases|fix|0
fix/quoted-aliases|fix test docs|0
fix/quoted-aliases|feat|1
docs/oracle-tests|docs|0
docs/oracle-tests|feat|1
chore/share-target|ci|0
chore/share-target|test|0
chore/share-target|feat|1
poc/session-picker|feat|0
poc/session-picker|refactor|0
bugfix/typo|fix|1
feature|feat|1
feature/|feat|1
development|feat|0
main||0
openwiki/update|docs|0
CASES
    if [ "$fails" -gt 0 ]; then
        echo "self-test: $fails case(s) failed" >&2
        return 1
    fi
    echo "self-test: all cases pass"
}

case "${1:-}" in
--self-test) self_test ;;
"") echo "usage: $0 <branch> [commit-type ...] | --self-test" >&2; exit 2 ;;
*) check "$@" ;;
esac
