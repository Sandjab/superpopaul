#!/usr/bin/env python3
"""
Popaul 🍿 — résout des adressages Peppol par fournées, via l'API /resolve/batch.

Lit une liste de Participant IDs (fichier texte, un par ligne, ou colonne d'un
CSV), les envoie à l'API `peppol_api` par paquets (≤ 500, la limite du serveur),
et écrit un CSV : existe ? / code PA / nom PA / pays / support EXTENDED-CTC-FR.
Gère l'auth par clé, les 429 (retry + backoff, `Retry-After` respecté) et
affiche une progression + un récap.

Dépendances : aucune (stdlib pure).

Exemples :
    # Fichier d'adressages (un PID par ligne) -> CSV sur stdout
    python popaul.py adressages.txt --url https://peppol.gavini.cloud --key MA_CLE

    # Colonne 'siren' d'un CSV -> fichier de sortie
    python popaul.py entreprises.csv --column siren -o resultats.csv \
           --url https://peppol.gavini.cloud --key MA_CLE

    # Depuis stdin, SML de test
    cat pids.txt | python popaul.py - --url http://127.0.0.1:8080 --key MA_CLE --test

La clé peut aussi venir de l'environnement (PEPPOL_API_KEY).
"""

from __future__ import annotations

import argparse
import csv
import json
import os
import sys
import time
import urllib.error
import urllib.request

BATCH_MAX = 500                      # limite serveur (/resolve/batch)
# Défaut volontairement bas : un batch coûte un jeton de rate-limit par adressage,
# donc grouper davantage n'accélère rien — mais un paquet de 500 tient la requête
# ouverte assez longtemps pour frôler le timeout. Monter via --batch-size au besoin.
BATCH_DEFAULT = 50
OUT_FIELDS = [
    "participant", "exists", "pa_code", "pa_name", "pa_country",
    "supports_extended_ctc_fr", "note",
]


DEFAULT_SCHEME = "iso6523-actorid-upis"
DEFAULT_ICD = "0225"


def eprint(*a):
    print(*a, file=sys.stderr, flush=True)


def canonical(pid: str) -> str:
    """Forme canonique du participant_id (comme le renvoie l'API).

    - 'scheme::icd:x' : déjà canonique, inchangé ;
    - 'icd:x' : scheme par défaut ajouté ;
    - adressage brut sans « : » (SIREN, SIREN_SIRET, SIREN_SIRET_CODEROUTAGE,
      SIREN_SUFFIXELIBRE) : préfixé de l'ICD français 0225 — sans lui, le hash
      SML porte sur la valeur nue et tout ressortirait « absent de Peppol ».
    Parité avec le client graphique (client/src-tauri/src/pid.rs::canonical)
    — tests miroir tests/test_popaul.py.
    """
    pid = pid.strip()
    if "::" in pid:
        return pid
    if ":" in pid:
        return f"{DEFAULT_SCHEME}::{pid}"
    return f"{DEFAULT_SCHEME}::{DEFAULT_ICD}:{pid}"


def progress_bar(done: int, total: int, extra: str = "") -> None:
    """Barre de progression sur stderr, mise à jour en place (si TTY)."""
    if not sys.stderr.isatty() or total <= 0:
        return
    width = 26
    frac = min(1.0, done / total)
    bar = "█" * int(width * frac) + "░" * (width - int(width * frac))
    sys.stderr.write(f"\r🍿 [{bar}] {frac*100:4.0f}%  {done}/{total}  {extra}   ")
    sys.stderr.flush()
    if done >= total:
        sys.stderr.write("\n")


# ---------------------------------------------------------------------------
# Entrée
# ---------------------------------------------------------------------------
def read_participants(path: str, column: str | None) -> list[str]:
    """Lit les PID depuis un fichier (ou '-' pour stdin). Sans --column : un PID
    par ligne (lignes vides et '#...' ignorées). Avec --column : colonne d'un CSV
    (nom d'en-tête ou index 0-based)."""
    fh = sys.stdin if path == "-" else open(path, encoding="utf-8-sig")
    try:
        if column is not None:
            reader = csv.reader(fh)
            rows = list(reader)
            if not rows:
                return []
            header = rows[0]
            try:
                idx = int(column)          # index numérique
                data_rows = rows            # pas d'en-tête à sauter
            except ValueError:
                if column not in header:
                    raise SystemExit(f"Colonne '{column}' absente de l'en-tête : {header}")
                idx = header.index(column)
                data_rows = rows[1:]        # saute la ligne d'en-tête
            pids = [r[idx].strip() for r in data_rows if len(r) > idx and r[idx].strip()]
        else:
            pids = [ln.strip() for ln in fh
                    if ln.strip() and not ln.lstrip().startswith("#")]
    finally:
        if fh is not sys.stdin:
            fh.close()
    return pids


def chunked(seq, size):
    for i in range(0, len(seq), size):
        yield seq[i:i + size]


# ---------------------------------------------------------------------------
# Appel API
# ---------------------------------------------------------------------------
def post_batch(url: str, key: str, pids: list[str], test: bool,
               timeout: float, max_retries: int) -> list[dict]:
    """POST /resolve/batch pour un paquet, avec retry/backoff sur 429/5xx/réseau.
    Renvoie la liste `results` (un item par PID, dans l'ordre)."""
    body = json.dumps({"participants": pids, "test": test}).encode()
    endpoint = f"{url.rstrip('/')}/resolve/batch"
    for attempt in range(max_retries + 1):
        req = urllib.request.Request(
            endpoint, data=body, method="POST",
            headers={"X-API-Key": key, "Content-Type": "application/json"},
        )
        try:
            with urllib.request.urlopen(req, timeout=timeout) as r:
                data = json.loads(r.read().decode())
            return data.get("results", [])
        except urllib.error.HTTPError as e:
            if e.code == 401:
                raise SystemExit("ERREUR 401 : clé d'API manquante ou invalide.")
            if e.code == 429 and attempt < max_retries:
                delay = _retry_after(e, attempt)
                eprint(f"  429 (rate limit) — nouvelle tentative dans {delay:.1f}s")
                time.sleep(delay)
                continue
            if 500 <= e.code < 600 and attempt < max_retries:
                time.sleep(_backoff(attempt))
                continue
            # Échec définitif : marque tout le paquet en erreur (sans casser le run).
            return [{"participant": p, "error": f"HTTP {e.code}"} for p in pids]
        except Exception as e:  # réseau/timeout
            if attempt < max_retries:
                time.sleep(_backoff(attempt))
                continue
            return [{"participant": p, "error": f"{type(e).__name__}: {e}"} for p in pids]
    return [{"participant": p, "error": "échec après retries"} for p in pids]


def _backoff(attempt: int) -> float:
    return min(30.0, 1.0 * (2 ** attempt))


def _retry_after(e: urllib.error.HTTPError, attempt: int) -> float:
    ra = e.headers.get("Retry-After") if e.headers else None
    if ra:
        try:
            return max(0.0, float(ra))
        except ValueError:
            pass
    return _backoff(attempt)


# ---------------------------------------------------------------------------
# Mise en forme CSV
# ---------------------------------------------------------------------------
def _fmt(v):
    if v is None:
        return ""
    if isinstance(v, bool):
        return "true" if v else "false"
    return str(v)


def to_row(item: dict, sent: str) -> dict:
    if "error" in item:
        return {"participant": item.get("participant", sent), "exists": "",
                "pa_code": "", "pa_name": "", "pa_country": "",
                "supports_extended_ctc_fr": "", "note": item["error"]}
    pa = item.get("pa") or {}
    return {
        "participant": item.get("participant_id", sent),
        "exists": _fmt(item.get("exists")),
        "pa_code": _fmt(pa.get("code")),
        "pa_name": _fmt(pa.get("name")),
        "pa_country": _fmt(pa.get("country")),
        "supports_extended_ctc_fr": _fmt(item.get("supports_extended_ctc_fr")),
        "note": _fmt(item.get("note")),
    }


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------
def main() -> int:
    ap = argparse.ArgumentParser(
        description="Popaul 🍿 — résout des adressages Peppol par fournées "
                    "(API /resolve/batch).")
    ap.add_argument("input", help="Fichier de PID (un par ligne) ou '-' pour stdin.")
    ap.add_argument("--url", required=True, help="Base URL de l'API, ex. https://peppol.gavini.cloud")
    ap.add_argument("--key", default=os.environ.get("PEPPOL_API_KEY"),
                    help="Clé d'API (sinon PEPPOL_API_KEY).")
    ap.add_argument("-o", "--output", default=None, help="CSV de sortie (défaut: stdout).")
    ap.add_argument("--column", default=None,
                    help="Entrée = CSV : nom d'en-tête ou index 0-based de la colonne des PID.")
    ap.add_argument("--batch-size", type=int, default=BATCH_DEFAULT,
                    help=f"Participants par requête (défaut {BATCH_DEFAULT}, max {BATCH_MAX}).")
    ap.add_argument("--test", action="store_true", help="Interroge le SML de test (SMK).")
    ap.add_argument("--resume", action="store_true",
                    help="Reprend : ignore les PID déjà résolus (avec réponse) dans -o, "
                         "et complète ce fichier au lieu de l'écraser.")
    ap.add_argument("--timeout", type=float, default=60.0, help="Timeout par requête (s).")
    ap.add_argument("--max-retries", type=int, default=4, help="Retries sur 429/5xx/réseau.")
    args = ap.parse_args()

    if not args.key:
        raise SystemExit("ERREUR : clé d'API requise (--key ou PEPPOL_API_KEY).")
    size = max(1, min(args.batch_size, BATCH_MAX))
    if args.batch_size > BATCH_MAX:
        eprint(f"[popaul] batch-size ramené à {BATCH_MAX} (limite serveur).")

    pids = read_participants(args.input, args.column)
    if not pids:
        raise SystemExit("Aucun adressage en entrée.")

    # Reprise : on lit le CSV de sortie existant, on retient les PID déjà résolus
    # (avec une réponse exists=true/false ; les erreurs seront réessayées) et on
    # complète le fichier plutôt que de l'écraser.
    append = False
    if args.resume:
        if not args.output:
            raise SystemExit("--resume nécessite -o/--output (le CSV à compléter).")
        if os.path.exists(args.output):
            done = set()
            with open(args.output, newline="", encoding="utf-8") as fh:
                for row in csv.DictReader(fh):
                    if row.get("exists", "") != "":
                        done.add(row.get("participant", ""))
            before = len(pids)
            pids = [p for p in pids if canonical(p) not in done]
            append = True
            eprint(f"[reprise] {len(done)} déjà résolus, {before - len(pids)} ignorés, "
                   f"{len(pids)} à traiter.")

    if not pids:
        eprint("✅ Rien à faire : tous les adressages sont déjà résolus.")
        return 0

    n_batches = (len(pids) + size - 1) // size
    eprint(f"🍿 Popaul : {len(pids)} adressages en {n_batches} fournée(s) de {size} "
           f"→ {args.url}")

    out_fh = (open(args.output, "a" if append else "w", newline="", encoding="utf-8")
              if args.output else sys.stdout)
    writer = csv.DictWriter(out_fh, fieldnames=OUT_FIELDS)
    if not append:
        writer.writeheader()

    counts = {"exists": 0, "absent": 0, "ext": 0, "error": 0}
    processed = 0
    try:
        for b, chunk in enumerate(chunked(pids, size), 1):
            if not sys.stderr.isatty():
                eprint(f"  fournée {b}/{n_batches} ({len(chunk)} adressages)…")
            results = post_batch(args.url, args.key, chunk, args.test,
                                 args.timeout, args.max_retries)
            # L'API renvoie un item par PID dans l'ordre ; on aligne par sécurité.
            for i, sent in enumerate(chunk):
                item = results[i] if i < len(results) else {"participant": sent, "error": "réponse tronquée"}
                row = to_row(item, sent)
                writer.writerow(row)
                if row["note"] and row["exists"] == "":
                    counts["error"] += 1
                elif row["exists"] == "true":
                    counts["exists"] += 1
                    if row["supports_extended_ctc_fr"] == "true":
                        counts["ext"] += 1
                elif row["exists"] == "false":
                    counts["absent"] += 1
            out_fh.flush()                         # résultats persistés au fil de l'eau
            processed += len(chunk)
            progress_bar(processed, len(pids), f"fournée {b}/{n_batches}")
    finally:
        if out_fh is not sys.stdout:
            out_fh.close()

    eprint(f"✅ Terminé : {counts['exists']} enregistrés "
           f"({counts['ext']} EXTENDED-CTC-FR), {counts['absent']} absents, "
           f"{counts['error']} en erreur.")
    if args.output:
        eprint(f"   Résultats → {args.output}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
