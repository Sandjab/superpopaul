# Super Popaul — conventions du sous-projet

- Sous-projet **autonome** : ne dépend d'aucun module Python du repo parent.
  La parité de comportement avec `popaul.py` (canonicalisation, format API)
  est vérifiée par tests, pas par import.
- Rust : modules étanches (`pid`, `config`, `store`, `modes`, `csv_io`, `api`,
  `telemetry`, `resolver`, `output`, `commands`). Toute logique métier est
  testable sans UI (`cargo test` dans `src-tauri/`).
- Frontend : vanilla HTML/CSS/JS, **pas de bundler ni de framework**.
  L'UI n'a aucune logique métier : elle invoque des commandes et affiche
  des événements.
- Sécurité UI : **jamais d'innerHTML avec des données dynamiques** (contenu
  CSV, messages d'erreur backend) — construire le DOM via le helper `h()`
  de `app.js` ou `textContent`. Un CSV est une entrée non fiable.
- Sécurité : les identifiants proxy ne sont JAMAIS écrits sur disque
  (test `config::proxy_creds_never_serialized` — ne pas le contourner).
- Texte UI et messages d'erreur en **français**.
- TDD : test d'abord pour toute logique Rust. Commits fréquents,
  format `feat(superpopaul): …` / `fix(superpopaul): …`.
