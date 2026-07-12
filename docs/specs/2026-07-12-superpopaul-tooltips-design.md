# Super Popaul — infobulles au survol (design)

Date : 2026-07-12 · Statut : validé

## Objectif

Ajouter des infobulles au survol partout où c'est justifié dans la GUI,
en particulier sur les étapes de configuration, pour expliquer les champs
et indicateurs dont la fonction ou l'unité n'est pas évidente d'après le
label seul.

## Mécanisme

Attribut HTML **`title` natif** — décision validée.
- Conforme à la convention existante (`title: "Exclure"` sur le ✕,
  `columns.js`).
- Zéro JS/CSS nouveau, zéro logique : compatible avec la règle « l'UI n'a
  aucune logique métier » du CLAUDE.md du sous-projet.
- Compromis accepté : délai d'affichage système (~1 s), style OS non
  personnalisable.

## Principe de sélection

Infobulle uniquement quand elle apporte une information non déductible :
- **Oui** : champs numériques avec unité ou effet non trivial, boutons à
  effet de bord (Calibrer, Sauvegarder…), indicateurs du cockpit.
- **Non** : éléments auto-explicatifs (dropzone, Suivant/Précédent) ou
  déjà expliqués par un texte `.muted` voisin (drag des colonnes, note
  proxy).

## Inventaire des infobulles

### Header — config YAML (`index.html`)

| Élément | Texte |
|---|---|
| Charger… | Charger une configuration YAML — reprend directement à l'étape Run. |
| Sauvegarder… | Sauvegarder la configuration en YAML. Attention : la clé API y est stockée en clair. |

### Étape 1 — Fichier (`index.html`)

| Élément | Texte |
|---|---|
| Colonne des adressages (label + select) | Colonne contenant les identifiants Peppol à résoudre (ex. SIREN/SIRET). |

### Étape 2 — Colonnes (`index.html` + `columns.js`)

| Élément | Texte |
|---|---|
| + Ajouter une colonne ⚡ | Ajouter un champ Peppol ou réintégrer une colonne exclue. |
| En-têtes ⚡ (colonnes Peppol, dynamique) | Champ calculé par l'API Peppol — les valeurs affichées sont un exemple. |

### Étape 3 — Sortie & API (`index.html`)

| Élément | Texte |
|---|---|
| Suffixer date/heure | Ajoute la date/heure au nom du fichier pour ne jamais écraser une sortie précédente. |
| URL | URL du service de résolution Peppol. |
| Clé | Clé d'authentification du service — bouton Tester pour la vérifier. |
| Tester | Envoie une requête de test pour vérifier l'URL, la clé et le proxy. |
| Concurrence | Nombre de requêtes simultanées vers l'API. « Calibrer » trouve la valeur optimale. |
| Taille de paquet | Nombre d'adressages envoyés par requête API (50 max). |
| Ancienneté refresh (jours) | Âge au-delà duquel un résultat en base est considéré périmé (utilisé par le mode Refresh). |
| Calibrer | Mesure le débit à plusieurs niveaux de concurrence et applique le meilleur. |

### Étape 4 — Run / cockpit (`index.html`)

| Élément | Texte |
|---|---|
| Sélecteur de mode | Full : tout re-résoudre · Reprise : seulement les manquants · Refresh : manquants + périmés. |
| Tuile 🟢 Dans Peppol | Part des adressages résolus présents dans l'annuaire Peppol. |
| Tuile 🇫🇷 CTC-FR | Part des adressages éligibles à la facturation électronique française (extension CTC-FR). |
| Tuile Débit | req/s : requêtes API · adr/s : adressages résolus par seconde. |
| Tuile Codes HTTP | Répartition des réponses API — 200 OK, 429 rate-limit, 0 erreur réseau. |
| Tuile Latence | Temps de réponse API en millisecondes (percentiles). |
| ETA | Temps restant estimé d'après le débit courant. |

Les infobulles sur les labels de l'étape 3 sont posées sur le `label` ET
le champ associé (le survol du champ est le geste le plus fréquent).

## Implémentation

- `index.html` : attributs `title` statiques (~15 lignes touchées).
- `columns.js` : `title` sur le bouton « + Ajouter une colonne » (statique,
  dans `index.html`) et sur les `th` Peppol dans `makeHeader()` via le
  helper `h()` — même pattern que le ✕ existant.
- Aucun fichier nouveau, aucun CSS, aucune logique.

## Vérification

- Lancement de l'app (`cargo tauri dev`) ou ouverture dans un navigateur,
  survol de chaque élément listé, constat visuel de l'infobulle.
- Pas de test Rust concerné : zéro logique métier (CLAUDE.md sous-projet).
