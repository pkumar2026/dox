#!/usr/bin/env bash
#
# Bump the Homebrew tap formula to a released version of dox.
#
# Workflow:
#   1. You've already tagged + pushed v0.X.Y to the main repo.
#   2. GitHub Actions has built the macOS arm64 binary and attached
#      it to the release.
#   3. Run: scripts/bump-formula.sh 0.X.Y
#
# What this does:
#   - Downloads the .sha256 sidecar file from the release.
#   - Updates `version` and `sha256` in the tap's Formula/dox.rb.
#   - Commits and pushes to origin in the tap repo.
#
set -euo pipefail

VERSION="${1:-}"
TAP_DIR="${DOX_TAP_DIR:-$HOME/code/homebrew-dox}"
REPO="${DOX_REPO:-pkumar2026/dox}"
FORMULA_PATH="Formula/dox.rb"

usage() {
    cat <<USAGE
usage: $(basename "$0") <version>

Updates the Homebrew tap formula to the given version by downloading
the SHA256 sum from the corresponding GitHub release, patching
$FORMULA_PATH in the tap repo, committing, and pushing.

Arguments:
  <version>   Version to bump to (e.g. 0.1.1 or v0.1.1).

Env vars:
  DOX_TAP_DIR  Path to the homebrew-dox repo (default: ~/code/homebrew-dox).
  DOX_REPO     GitHub owner/repo for the dox project (default: pkumar2026/dox).
  PUSH         Set to 0 to skip pushing the tap commit. Default: 1.
USAGE
    exit 2
}

die() {
    echo "error: $*" >&2
    exit 1
}

if [[ -z "$VERSION" || "$VERSION" == "-h" || "$VERSION" == "--help" ]]; then
    usage
fi

VERSION="${VERSION#v}"
TAG="v${VERSION}"

command -v gh >/dev/null || die "gh CLI not found. install with: brew install gh"
[[ -d "$TAP_DIR" ]] || die "tap dir not found: $TAP_DIR. set DOX_TAP_DIR or clone the tap first."
[[ -f "$TAP_DIR/$FORMULA_PATH" ]] || die "formula not found at $TAP_DIR/$FORMULA_PATH"

echo "Downloading SHA file for $REPO@$TAG"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
gh release download "$TAG" --repo "$REPO" -p '*.sha256' --dir "$WORK" --clobber >/dev/null \
    || die "couldn't download release assets — has the build finished? (gh run list -R $REPO)"

SHA_FILE="$WORK/dox-${TAG}-aarch64-apple-darwin.tar.gz.sha256"
[[ -f "$SHA_FILE" ]] || die "expected SHA file missing: $(basename "$SHA_FILE")"

SHA=$(awk '{print $1}' "$SHA_FILE")
[[ "$SHA" =~ ^[a-f0-9]{64}$ ]] || die "aarch64 SHA didn't parse: $SHA"

echo "  aarch64: $SHA"

FORMULA="$TAP_DIR/$FORMULA_PATH"
echo "Patching $FORMULA"

# perl is portable across macOS BSD sed vs GNU sed.
perl -i -pe 's/^(\s*version\s+)"[^"]+"/${1}"'"$VERSION"'"/' "$FORMULA"
perl -i -pe 's/^(\s*sha256\s+)"[A-Za-z0-9_]*"/${1}"'"$SHA"'"/' "$FORMULA"

grep -q "$SHA" "$FORMULA" || die "SHA was not written to formula"
grep -q "version \"$VERSION\"" "$FORMULA" || die "version was not written to formula"

if command -v brew >/dev/null; then
    echo "Auditing formula"
    brew audit --strict --formula "$FORMULA" 2>&1 | sed 's/^/  /' || true
fi

git -C "$TAP_DIR" diff --quiet "$FORMULA_PATH" && die "no changes detected — was this already bumped?"

echo "Committing"
git -C "$TAP_DIR" add "$FORMULA_PATH"
git -C "$TAP_DIR" commit -m "feat: bump dox to $VERSION"

if [[ "${PUSH:-1}" == "1" ]]; then
    echo "Pushing"
    git -C "$TAP_DIR" push origin HEAD
    echo
    echo "Done. Users can now run: brew upgrade dox"
else
    echo
    echo "Done (PUSH=0). Push manually with: git -C $TAP_DIR push"
fi
