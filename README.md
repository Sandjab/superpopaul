# Super Popaul 🍿

Application graphique standalone (Windows + macOS) de résolution Peppol en masse :
un CSV d'adressages en entrée, un CSV enrichi en sortie (existe dans Peppol,
code PA, pays PA, support EXTENDED-CTC-FR), via l'API REST `peppol_api`
(la même que le client CLI `popaul.py` à la racine du repo).

## Points clés
- **Wizard 3 étapes** : fichier d'entrée → colonnes de sortie → run. La sortie
  (répertoire + suffixe), l'API et le proxy se règlent dans le panneau ⚙.
- **Réglages auto-persistés** (`superpopaul.yaml`, dossier données utilisateur) :
  lus au démarrage, écrits à la fermeture du panneau ⚙. La clé API y est
  stockée ; les identifiants proxy **jamais**.
- **Cache SQLite global** (dossier données utilisateur) : chaque adressage unique
  est résolu une fois ; modes **full / reprise / refresh** (seuil d'ancienneté).
- **Profils de chargement YAML** sauvegardés/chargés explicitement : fichier
  d'entrée (chemin relatif au YAML), colonne des adressages, colonnes de
  sortie. Ni clé API ni réglages ; les anciennes configs complètes restent
  chargeables (seul le profil en est repris).
- **Cockpit temps réel** : ring de progression + ETA, % Peppol, % CTC-FR,
  débits (req/s et adressages/s), codes HTTP, latences p50/p90/p99.
- **Pause/reprise** à chaud et entre sessions (détection de run incomplet).
- Erreurs intelligentes : 401 → suspension + ressaisie de clé ; 429 → backoff
  adaptatif (AIMD) ; 5xx en rafale → circuit breaker avec re-test automatique.

## Développement

```bash
cd superpopaul/src-tauri
cargo test          # logique métier (aucune UI requise)
cargo tauri dev     # app en mode dev
cargo tauri build   # binaire de distribution
```

## Distribution
Binaires **non signés** : la procédure d'ouverture (Gatekeeper macOS,
SmartScreen Windows) est détaillée dans `NOTICE-OUVERTURE.md`.
macOS : build local. Windows : GitHub Actions (`.github/workflows/superpopaul-windows.yml`).

## Spec & plan
- Spec : `../docs/superpowers/specs/2026-07-12-super-popaul-design.md`
- Plan : `../docs/superpowers/plans/2026-07-12-super-popaul.md`
