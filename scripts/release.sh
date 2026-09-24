#!/usr/bin/env bash
#
# Prisme.ai Desktop — release manager.
#
#   ./scripts/release.sh check           # prerequisites + which secrets are set
#   ./scripts/release.sh secrets         # interactively set signing/notarization secrets
#   ./scripts/release.sh publish 0.1.0   # bump version, tag, push -> triggers the signed CI release
#
# Nothing sensitive is ever echoed. Certificates/keys are read from a file path
# you provide (drag the file into the terminal) and base64-encoded on the fly.
#
set -euo pipefail

REPO="${PRISME_DESKTOP_REPO:-prismeai/desktop}"

# --- pretty output ----------------------------------------------------------
if [ -t 1 ]; then
  B=$'\033[1m'; DIM=$'\033[2m'; G=$'\033[32m'; Y=$'\033[33m'; R=$'\033[31m'; C=$'\033[36m'; N=$'\033[0m'
else
  B=""; DIM=""; G=""; Y=""; R=""; C=""; N=""
fi
say()  { printf '%s\n' "$*"; }
head() { printf '\n%s\n' "${B}${C}== $* ==${N}"; }
ok()   { printf '%s\n' "${G}✓${N} $*"; }
warn() { printf '%s\n' "${Y}!${N} $*"; }
die()  { printf '%s\n' "${R}✗ $*${N}" >&2; exit 1; }

require() {
  command -v gh >/dev/null 2>&1 || die "GitHub CLI 'gh' not found (https://cli.github.com)."
  gh auth status >/dev/null 2>&1 || die "Not logged in: run 'gh auth login'."
  gh repo view "$REPO" >/dev/null 2>&1 || die "No access to $REPO."
}

secret_is_set() { gh secret list --repo "$REPO" 2>/dev/null | awk '{print $1}' | grep -qx "$1"; }

# Ask before overwriting a secret that already exists.
should_set() {
  local name="$1"
  if secret_is_set "$name"; then
    printf '%s' "  ${DIM}${name} already set — replace it? [y/N] ${N}"
    read -r ans
    [ "${ans:-}" = "y" ] || [ "${ans:-}" = "Y" ]
  else
    return 0
  fi
}

# set a plain value (typed, hidden). Blank input = skip.
set_plain() {
  local name="$1" prompt="$2"
  should_set "$name" || { say "  ${DIM}skipped${N}"; return; }
  printf '%s' "  ${prompt}: "
  read -r value
  [ -n "$value" ] || { warn "  empty — skipped"; return; }
  gh secret set "$name" --repo "$REPO" --body "$value" >/dev/null && ok "$name set"
}

# set a secret value hidden (no echo). Blank = skip.
set_hidden() {
  local name="$1" prompt="$2"
  should_set "$name" || { say "  ${DIM}skipped${N}"; return; }
  printf '%s' "  ${prompt}: "
  read -rs value; printf '\n'
  [ -n "$value" ] || { warn "  empty — skipped"; return; }
  gh secret set "$name" --repo "$REPO" --body "$value" >/dev/null && ok "$name set"
}

# base64-encode a file and store it. Blank path = skip.
set_file_b64() {
  local name="$1" prompt="$2"
  should_set "$name" || { say "  ${DIM}skipped${N}"; return; }
  printf '%s' "  ${prompt} (path, or drag the file here): "
  read -r path
  path="${path//\'/}"; path="${path//\"/}"; path="${path/#\~/$HOME}"
  path="$(printf '%s' "$path" | sed -e 's/[[:space:]]*$//')"
  [ -n "$path" ] || { warn "  empty — skipped"; return; }
  [ -f "$path" ] || die "  file not found: $path"
  base64 -i "$path" | gh secret set "$name" --repo "$REPO" >/dev/null && ok "$name set (from $(basename "$path"))"
}

# ---------------------------------------------------------------------------
cmd_check() {
  require
  head "Repository"
  gh repo view "$REPO" --json nameWithOwner,visibility,url \
    --jq '"  " + .nameWithOwner + "  (" + .visibility + ")  " + .url'

  head "Required secrets"
  local updater=(TAURI_SIGNING_PRIVATE_KEY TAURI_SIGNING_PRIVATE_KEY_PASSWORD)
  local apple=(APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY APPLE_TEAM_ID APPLE_API_ISSUER APPLE_API_KEY APPLE_API_KEY_P8_BASE64)
  local windows=(WINDOWS_SIGN_TENANT_ID WINDOWS_SIGN_CLIENT_ID WINDOWS_SIGN_CLIENT_SECRET WINDOWS_SIGN_ENDPOINT WINDOWS_SIGN_ACCOUNT WINDOWS_SIGN_PROFILE)
  local group name
  for group in "Updater:${updater[*]}" "macOS:${apple[*]}" "Windows (optional):${windows[*]}"; do
    printf '  %s\n' "${B}${group%%:*}${N}"
    for name in ${group#*:}; do
      if secret_is_set "$name"; then printf '    %s %s\n' "${G}✓${N}" "$name"; else printf '    %s %s\n' "${R}·${N}" "$name"; fi
    done
  done
  say ""
  say "  Run ${B}./scripts/release.sh secrets${N} to set the missing ones."
}

# ---------------------------------------------------------------------------
cmd_secrets() {
  require
  say "${B}Setting release secrets for ${REPO}${N}"
  say "${DIM}Blank input = skip. Existing secrets are only touched if you confirm.${N}"

  head "Updater (required — signs the auto-update feed)"
  say "  ${DIM}Where: generated once with 'tauri signer generate'. Private key on the"
  say "  maintainer machine at ~/.tauri/prismeai-desktop-updater.key.${N}"
  if [ -f "$HOME/.tauri/prismeai-desktop-updater.key" ]; then
    if should_set TAURI_SIGNING_PRIVATE_KEY; then
      base64 -i "$HOME/.tauri/prismeai-desktop-updater.key" >/dev/null 2>&1 # validate readable
      gh secret set TAURI_SIGNING_PRIVATE_KEY --repo "$REPO" < "$HOME/.tauri/prismeai-desktop-updater.key" >/dev/null && ok "TAURI_SIGNING_PRIVATE_KEY set"
    fi
    if should_set TAURI_SIGNING_PRIVATE_KEY_PASSWORD; then
      printf '' | gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD --repo "$REPO" >/dev/null && ok "TAURI_SIGNING_PRIVATE_KEY_PASSWORD set (empty)"
    fi
  else
    warn "  ~/.tauri/prismeai-desktop-updater.key not found — set TAURI_SIGNING_PRIVATE_KEY manually."
  fi

  head "macOS — Developer ID certificate (signs the app)"
  say "  ${DIM}Where to get it:"
  say "   1) Keychain Access ▸ Certificate Assistant ▸ Request a Certificate… ▸ save CSR."
  say "   2) developer.apple.com ▸ Certificates ▸ + ▸ 'Developer ID Application' ▸ upload CSR ▸ download."
  say "   3) Double-click the .cer, then in Keychain Access right-click ▸ Export ▸ .p12 (set a password).${N}"
  set_file_b64 APPLE_CERTIFICATE          "Path to the exported .p12"
  set_hidden   APPLE_CERTIFICATE_PASSWORD "The .p12 password"
  say "  ${DIM}Signing identity = the certificate's exact name; Team ID on developer.apple.com/account (Membership).${N}"
  set_plain    APPLE_SIGNING_IDENTITY     "APPLE_SIGNING_IDENTITY  e.g. 'Developer ID Application: Your Org (TEAMID)'"
  set_plain    APPLE_TEAM_ID              "APPLE_TEAM_ID  (10 chars)"

  head "macOS — notarization (App Store Connect API key)"
  say "  ${DIM}Where: appstoreconnect.apple.com ▸ Users and Access ▸ Integrations ▸"
  say "  App Store Connect API ▸ + Generate (role Developer). Note the Issuer ID and"
  say "  Key ID, and download the .p8 (only once).${N}"
  set_plain    APPLE_API_ISSUER          "APPLE_API_ISSUER  (Issuer ID, a UUID)"
  set_plain    APPLE_API_KEY             "APPLE_API_KEY  (Key ID, 10 chars)"
  set_file_b64 APPLE_API_KEY_P8_BASE64   "Path to the AuthKey_XXXX.p8"

  head "Windows — code signing (optional: Azure Trusted Signing)"
  printf '%s' "  Configure Windows signing now? [y/N] "
  read -r do_win
  if [ "${do_win:-}" = "y" ] || [ "${do_win:-}" = "Y" ]; then
    say "  ${DIM}Where: Azure Portal ▸ Trusted Signing account. Create an App registration"
    say "  (Entra ID) with a client secret and the 'Trusted Signing Certificate Profile"
    say "  Signer' role on the account. Endpoint looks like https://<region>.codesigning.azure.net.${N}"
    set_plain  WINDOWS_SIGN_TENANT_ID     "AZURE tenant id"
    set_plain  WINDOWS_SIGN_CLIENT_ID     "AZURE app (client) id"
    set_hidden WINDOWS_SIGN_CLIENT_SECRET "AZURE client secret"
    set_plain  WINDOWS_SIGN_ENDPOINT      "Trusted Signing endpoint URL"
    set_plain  WINDOWS_SIGN_ACCOUNT       "Trusted Signing account name"
    set_plain  WINDOWS_SIGN_PROFILE       "Certificate profile name"
    warn "  Wiring: set bundle.windows.signCommand in tauri.conf.json to trusted-signing-cli"
    warn "  and pass these as env in release.yml. See RELEASING.md §4."
  else
    say "  ${DIM}Skipped — Windows .msi/.exe will ship unsigned (SmartScreen warns).${N}"
  fi

  head "Done"
  say "  Review: ${B}./scripts/release.sh check${N}"
  say "  Release: ${B}./scripts/release.sh publish 0.1.0${N}"
}

# ---------------------------------------------------------------------------
cmd_publish() {
  require
  local ver="${1#v}"
  printf '%s' "$ver" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.]+)?$' \
    || die "Version must be semver, e.g. 0.1.0 (got '$ver')."

  local root; root="$(cd "$(dirname "$0")/.." && pwd)"
  cd "$root"
  [ -z "$(git status --porcelain)" ] || die "Working tree not clean — commit or stash first."
  [ "$(git rev-parse --abbrev-ref HEAD)" = "main" ] || warn "Not on 'main' branch."

  head "Bumping version to $ver"
  npm version "$ver" --no-git-tag-version --allow-same-version >/dev/null
  node -e "const f='src-tauri/tauri.conf.json';const fs=require('fs');const j=JSON.parse(fs.readFileSync(f));j.version='$ver';fs.writeFileSync(f,JSON.stringify(j,null,2)+'\n')"
  sed -i '' -E "0,/^version = \".*\"/s//version = \"$ver\"/" src-tauri/Cargo.toml
  ( cd src-tauri && cargo update -p prismeai-desktop --precise "$ver" --quiet 2>/dev/null || true )
  ok "package.json, tauri.conf.json, Cargo.toml -> $ver"

  head "Commit + tag"
  git add package.json package-lock.json src-tauri/tauri.conf.json src-tauri/Cargo.toml src-tauri/Cargo.lock 2>/dev/null || true
  git commit -q -m "chore: release v$ver"
  git tag "v$ver"
  git push origin main --quiet
  git push origin "v$ver" --quiet
  ok "pushed v$ver"

  head "Next"
  say "  CI is building signed installers now:"
  say "    ${C}https://github.com/$REPO/actions${N}"
  say "  When it finishes, ${B}publish the draft Release${N} (Releases tab) to make the"
  say "  download links live and activate auto-update:"
  say "    ${C}https://github.com/$REPO/releases${N}"
}

# ---------------------------------------------------------------------------
case "${1:-}" in
  check)   cmd_check ;;
  secrets) cmd_secrets ;;
  publish) shift; cmd_publish "${1:?usage: publish <version>, e.g. 0.1.0}" ;;
  *)
    say "${B}Prisme.ai Desktop — release manager${N}"
    say ""
    say "  ./scripts/release.sh check            prerequisites + which secrets are set"
    say "  ./scripts/release.sh secrets          interactively set signing/notarization secrets"
    say "  ./scripts/release.sh publish <ver>    bump + tag + push -> signed CI release"
    say ""
    say "  Repo: ${REPO}  (override with PRISME_DESKTOP_REPO=owner/name)"
    ;;
esac
