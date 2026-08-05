#!/usr/bin/env bash
# release-ci.sh — mécanique déterministe d'une release superpopaul :
# tag → push → attente du run CI du tag → pose des notes (si fournies).
#
# Usage : release-ci.sh vX.Y.Z [notes.md]
#
# Ce script ne fait PAS : le bump de version (tauri.conf.json / Cargo.toml /
# Cargo.lock, commit chore) ni la rédaction des notes — ces deux étapes
# demandent du jugement et restent au modèle ou à l'humain.
set -euo pipefail

TAG="${1:?usage: release-ci.sh vX.Y.Z [notes.md]}"
NOTES="${2:-}"

[[ "$TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "tag invalide : $TAG (attendu vX.Y.Z)" >&2; exit 1; }
[ -z "$(git status --porcelain)" ] || { echo "arbre non propre — commit d'abord" >&2; exit 1; }
[ -z "$NOTES" ] || [ -f "$NOTES" ] || { echo "fichier de notes introuvable : $NOTES" >&2; exit 1; }

git tag "$TAG"
git push origin main "$TAG"

# Le workflow windows.yml se déclenche sur le tag ; on cherche son run.
echo "Recherche du run CI du tag $TAG…"
RUN_ID=""
for _ in 1 2 3 4 5 6; do
  sleep 10
  RUN_ID=$(gh run list --limit 10 --json databaseId,headBranch \
    -q ".[] | select(.headBranch==\"$TAG\") | .databaseId" | head -1)
  [ -n "$RUN_ID" ] && break
done
[ -n "$RUN_ID" ] || { echo "run CI du tag introuvable après 60 s" >&2; exit 1; }

echo "Run $RUN_ID — attente (interval 60 s)…"
if gh run watch "$RUN_ID" --exit-status --interval 60 > /dev/null 2>&1; then
  echo "CI OK"
else
  echo "CI ÉCHEC — voir : gh run view $RUN_ID" >&2
  exit 1
fi

if [ -n "$NOTES" ]; then
  gh release edit "$TAG" --notes-file "$NOTES"
  echo "— Notes publiées :"
  gh release view "$TAG" --json body -q .body
else
  echo "Rappel : notes humaines à poser — gh release edit $TAG --notes-file <notes.md>"
fi
