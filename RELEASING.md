# Releasing Prisme.ai Desktop

Signed, notarized builds are produced by CI on a version tag. Distribution model
is **Developer ID + notarization** (direct `.dmg`, Slack-style) with a
self-update feed on **GitHub Releases**. You set a handful of secrets once, then
every release is just a git tag.

---

## 0. Where secrets go

GitHub → your repo (`prismeai/desktop`) → **Settings** → **Secrets and variables**
→ **Actions** → **New repository secret**. Add each name below with its value.

> Tip: for org-wide reuse, use an **Organization** secret instead (Org Settings →
> Secrets and variables → Actions), scoped to this repo.

---

## 1. macOS — Developer ID certificate (the signing cert)

Goal: a `Developer ID Application` certificate exported as a password-protected
`.p12`, then base64 for CI.

1. **Make a signing request (CSR).** Open **Keychain Access** → menu
   **Keychain Access → Certificate Assistant → Request a Certificate From a
   Certificate Authority…** → enter your email, leave "CA Email" empty, choose
   **Saved to disk** → save `CertificateSigningRequest.certSigningRequest`.
2. **Create the cert.** Go to
   <https://developer.apple.com/account/resources/certificates/list> →
   **+** → choose **Developer ID Application** → Continue → upload the CSR →
   Continue → **Download** the `.cer`.
3. **Install it.** Double-click the `.cer` → it lands in Keychain Access under
   **login → My Certificates** as `Developer ID Application: <Your Org> (TEAMID)`.
4. **Export to .p12.** In Keychain Access, right-click that certificate →
   **Export…** → format **Personal Information Exchange (.p12)** → set a strong
   password (remember it).
5. **Base64 it** (macOS):
   ```bash
   base64 -i Certificates.p12 | pbcopy
   ```

Secrets to set:
| Secret | Value |
|---|---|
| `APPLE_CERTIFICATE` | the base64 from step 5 |
| `APPLE_CERTIFICATE_PASSWORD` | the .p12 password from step 4 |
| `APPLE_SIGNING_IDENTITY` | `Developer ID Application: <Your Org> (TEAMID)` (the cert's exact name) |
| `APPLE_TEAM_ID` | your 10-char Team ID (Membership page: <https://developer.apple.com/account#MembershipDetailsCard>) |

---

## 2. macOS — notarization (App Store Connect API key)

More robust than Apple-ID + app-password (no 2FA prompts in CI).

1. Go to <https://appstoreconnect.apple.com/access/integrations/api> (App Store
   Connect → **Users and Access** → **Integrations** → **App Store Connect API**).
2. Note the **Issuer ID** (UUID at the top).
3. **Generate API Key** → name it `desktop-notarization`, role **Developer** →
   Generate. Note the **Key ID** (10 chars). **Download the `.p8` now** (you only
   get one chance).
4. Base64 the key:
   ```bash
   base64 -i AuthKey_XXXXXXXXXX.p8 | pbcopy
   ```

Secrets to set:
| Secret | Value |
|---|---|
| `APPLE_API_ISSUER` | the Issuer ID (UUID) |
| `APPLE_API_KEY` | the Key ID (10 chars) |
| `APPLE_API_KEY_P8_BASE64` | the base64 of the `.p8` |

---

## 3. Updater signing key

The app verifies updates against a public key baked into `tauri.conf.json`. CI
signs each build with the matching **private** key.

- The keypair was generated with `tauri signer generate`; the private key is at
  `~/.tauri/prismeai-desktop-updater.key` on the maintainer's machine.
- Put its **contents** in a secret (do not commit it):

| Secret | Value |
|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | the full contents of the private key file |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | empty (the key has no password) |

> Losing this key means clients can no longer verify updates — back it up in
> your password manager / secret store.

---

## 4. Windows signing (recommended: Azure Trusted Signing)

Traditional OV/EV certificates now ship on **hardware tokens** → not usable in
CI. The industrial, CI-friendly options (cloud signing, no token):

- **Azure Trusted Signing** (~US$10/mo) — Microsoft's cloud signing. Recommended.
- **SSL.com eSigner** or **DigiCert KeyLocker** — cloud HSM alternatives.
- **SignPath.io** — free tier for open-source.

Wiring (once you pick one): set `bundle.windows.signCommand` in
`tauri.conf.json` to the provider's CLI and add its credentials as `AZURE_*`
(or provider-specific) secrets, then the same `release.yml` signs the `.msi`.

Until then, Windows artifacts ship **unsigned** (SmartScreen shows a warning).
Everything else (build, updater) still works.

---

## 5. Cut a release

Once the secrets above are set:

```bash
# bump the version in src-tauri/tauri.conf.json + Cargo.toml + package.json first
git tag v0.1.0
git push origin v0.1.0
```

The `Release` workflow then:
1. builds macOS (universal) + Windows,
2. signs + notarizes macOS (Developer ID), signs Windows (if configured),
3. generates `latest.json` (updater feed) signed with the updater key,
4. creates a **draft GitHub Release** with the `.dmg`, Windows installer, and
   `latest.json` attached.

**Review the draft, then Publish it.** Publishing matters: the updater endpoint
is `…/releases/latest/download/latest.json`, and GitHub only serves `latest`
from a **published** (non-draft, non-prerelease) release. Once published,
existing installs auto-update on next launch.

---

## Secret checklist

- [ ] `APPLE_CERTIFICATE`
- [ ] `APPLE_CERTIFICATE_PASSWORD`
- [ ] `APPLE_SIGNING_IDENTITY`
- [ ] `APPLE_TEAM_ID`
- [ ] `APPLE_API_ISSUER`
- [ ] `APPLE_API_KEY`
- [ ] `APPLE_API_KEY_P8_BASE64`
- [ ] `TAURI_SIGNING_PRIVATE_KEY`
- [ ] `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (empty)
- [ ] Windows signing secrets (when the certificate is set up)
