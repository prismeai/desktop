# Production TODO (interne)

> Suivi interne — **ne pas mettre dans le README client**. Déplacé hors du README
> lors du nettoyage product-facing.

Ce qui reste à faire avant une distribution production du client desktop :

- [ ] **Signature de code + notarisation** : Apple Developer ID (macOS), certificat EV
      Windows.
- [ ] **Auto-update** : `tauri-plugin-updater` + feed de release (`latest.json` signé,
      déjà émis par `.github/workflows/release.yml`).
- [ ] **Finitions natives** : notifications, tray, deep links, single-instance.
- [ ] **Auth external-IdP** : flux system-browser RFC 8252 pour les clients fédérés à un
      IdP (Azure AD / Okta…) qui bloque les webviews embarquées.
