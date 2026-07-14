# Super Popaul — conventions du projet

- Projet **indépendant**, issu du monorepo peppolstat (split du 2026-07-14).
  La parité stricte avec le client CLI `popaul.py` est abandonnée : la
  canonicalisation et le format API évoluent ici librement, couverts par
  les tests Rust existants (`pid`, `api`).
- Rust : modules étanches (`pid`, `config`, `store`, `modes`, `csv_io`, `api`,
  `telemetry`, `resolver`, `output`, `commands`). Toute logique métier est
  testable sans UI (`cargo test` dans `src-tauri/`).
- Frontend : vanilla HTML/CSS/JS, **pas de bundler ni de framework**.
  L'UI n'a aucune logique métier : elle invoque des commandes et affiche
  des événements.
  Dérogation unique : SortableJS 1.15.6 vendorisé (`src/vendor/Sortable.min.js`,
  MIT, fichier seul) pour le drag des colonnes de l'étape 2 — le DnD HTML5 est
  avalé par le handler drag-drop Tauri (requis pour le drop de fichier), et le
  mode `forceFallback` de Sortable donne un drag pointeur animé qu'un
  équivalent maison ne justifiait pas de réécrire.
- Sécurité UI : **jamais d'innerHTML avec des données dynamiques** (contenu
  CSV, messages d'erreur backend) — construire le DOM via le helper `h()`
  de `app.js` ou `textContent`. Un CSV est une entrée non fiable.
- Sécurité : les identifiants proxy ne sont JAMAIS écrits sur disque
  (test `config::proxy_creds_never_serialized` — ne pas le contourner).
- Texte UI et messages d'erreur en **français**.
- TDD : test d'abord pour toute logique Rust. Commits fréquents,
  format `feat(superpopaul): …` / `fix(superpopaul): …`.
