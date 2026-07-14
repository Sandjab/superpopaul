#!/usr/bin/env python3
"""
Peppol Resolver REST API — expose `peppol_resolver.py` (SML+SMP direct) derrière
une petite API HTTP protégée par clé, pensée pour tourner sur un VPS perso
derrière nginx (TLS) et lancée par systemd.

Pour un adressage donné (Participant ID Peppol), l'endpoint principal répond
simplement :
  * s'il existe (enregistré dans le SML, un SMP le route) ;
  * le code de la PA (= Common Name du certificat de l'Access Point) ;
  * le nom de la PA (= Organization du certificat) ;
  * si la PA supporte le format UBL EXTENDED-CTC-FR (facture structurée
    principale du PASR France §6.1.c).

Toute la résolution (DNS NAPTR sur le SML, requêtes SMP, parsing XML +
certificat X.509) est faite par `peppol_resolver.py`. Aucune API tierce n'est
appelée.

Sécurité / robustesse :
  * Authentification par clé d'API (header `X-API-Key` ou `Authorization:
    Bearer <clé>`), plusieurs clés possibles (une par client), révocables.
  * Rate-limiting token-bucket par clé (protège l'annuaire Peppol et le VPS).
  * Concurrence bornée (les résolutions sont I/O-bound vers le réseau Peppol).
  * `/health` non authentifié pour le healthcheck du reverse-proxy / monitoring.
  * Doc interactive Swagger UI sur `/docs`, spec OpenAPI sur `/openapi.json`.

Endpoints :
  GET /health                         -> {"status":"ok"}                (public)
  GET /openapi.json                   -> spec OpenAPI 3.0               (public)
  GET /docs                           -> Swagger UI                      (public)
  GET /resolve/<participant>          -> réponse simple                 (clé)
  GET /resolve?participant=<id>       -> idem (participant en query)     (clé)
      &test=1        interroge le SML de test (SMK) au lieu de la prod
      &detail=full   renvoie en plus le JSON complet de peppol_resolver
  POST /resolve/batch                 -> résout une liste (max 500)      (clé)
      corps JSON : {"participants": ["0225:x", ...], "test": false}
      -> {"count": N, "results": [ <réponse simple> | {participant,error} ]}
  GET /limits                         -> quota de la clé présentée       (clé)
      -> {"current": jetons restants, "limit": req/min, "burst": pic,
          "retry_after": secondes avant de pouvoir rappeler (0 si dispo)}
      Gratuit (ne prélève aucun jeton). `current` < 0 = découvert.

Config (CLI ou variables d'environnement) :
  --host           PEPPOL_API_HOST        (défaut 127.0.0.1)
  --port           PEPPOL_API_PORT        (défaut 8080)
  --keys           PEPPOL_API_KEYS        clés séparées par des virgules
  --keys-file      PEPPOL_API_KEYS_FILE   fichier: une clé/ligne (voir format)
  --rate-limit     PEPPOL_API_RATE_LIMIT  requêtes/min par DÉFAUT (0 = illimité)
  --rate-burst     PEPPOL_API_RATE_BURST  pic par défaut (défaut = rate-limit)

Format des clés (fichier `--keys-file` OU entrées `--keys` séparées par des
virgules) — champs séparés par des espaces :

    label=CLE [rate] [burst]

  * label=CLE : nom du client (pour les logs) + la clé ; le label est
    optionnel (`CLE` seule marche aussi).
  * rate  (optionnel) : requêtes/min PROPRES à cette clé ; omis ou `-` = défaut
    global (--rate-limit) ; `0` = illimité pour cette clé.
  * burst (optionnel) : pic propre ; omis = égal au rate de la clé.

  Exemples de lignes de fichier de clés :
    # client courant : hérite du défaut global
    monclient=Xy9...abc
    # client premium : 600 req/min, pic 100
    partenaire=Zk3...def   600   100
    # sonde interne : illimitée
    interne=Qw7...ghi      0
  --max-concurrency PEPPOL_API_MAX_CONCURRENCY  résolutions simultanées
  --dns-server     PEPPOL_API_DNS_SERVER   serveur DNS des NAPTR (ex. 1.1.1.1)
  --doh            PEPPOL_API_DOH          endpoint DNS-over-HTTPS
  --dns-fallback   PEPPOL_API_DNS_FALLBACK résolveur de secours (défaut 8.8.8.8)
  --proxy / --ca-bundle / --insecure
        transmis tels quels à peppol_resolver.configure_network()

Générer une clé :   python peppol_api.py --gen-key
Lancer :            python peppol_api.py --keys "$(cat key.txt)" --port 8080

Dépendances : celles de peppol_resolver.py (dnspython, cryptography).
"""

from __future__ import annotations

import argparse
import hmac
import json
import logging
import os
import secrets
import sys
import threading
import time
import urllib.parse
from concurrent.futures import ThreadPoolExecutor
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any, NamedTuple

import peppol_resolver as resolver

log = logging.getLogger("peppol_api")

# Doctype UBL EN16931 France EXTENDED-CTC-FR — la facture structurée principale
# du PASR France (§6.1.c). Défini en local (et non importé de peppol_report, un
# module d'analytics de ~139 Ko) : le déploiement n'a ainsi besoin que de
# peppol_api.py + peppol_resolver.py. La valeur DOIT rester identique à
# peppol_report.FR_CTC_PRIMARY_INVOICE — un test garde-fou le vérifie.
FR_CTC_PRIMARY_INVOICE = (
    "urn:oasis:names:specification:ubl:schema:xsd:Invoice-2::Invoice"
    "##urn:cen.eu:en16931:2017#conformant"
    "#urn:peppol:france:billing:extended:1.0::2.1"
)


class KeyConfig(NamedTuple):
    """Config d'une clé d'API : label (pour les logs) + rate-limit propre.
    `rate` en requêtes/min (0 = illimité) ; `burst` = pic autorisé. Ces valeurs
    sont déjà résolues (les défauts globaux ont été appliqués au chargement)."""
    label: str
    rate: int
    burst: int


# --- Configuration résolue au démarrage (voir build_config) ------------------
API_KEYS: dict[str, KeyConfig] = {}    # clé secrète -> KeyConfig
DEFAULT_RATE_PER_MIN = 60              # défaut appliqué aux clés sans rate propre
DEFAULT_BURST = 60
MAX_CONCURRENCY = 64                   # résolutions simultanées (borne le pool batch)
_SEM: threading.BoundedSemaphore | None = None
_CONCURRENCY_TIMEOUT = 30.0            # attente max d'un slot avant 503

# /resolve/batch : nombre max de participants par requête, et taille max du corps.
BATCH_MAX = 500
BATCH_BODY_MAX_BYTES = 64 * 1024


# ---------------------------------------------------------------------------
# Clés d'API
# ---------------------------------------------------------------------------
def _iter_specs(inline: str | None, keys_file: str | None):
    """Rend les « specs » de clés (listes de champs) depuis --keys (entrées
    séparées par des virgules) et --keys-file (une spec/ligne, `#` commentaire).
    Chaque spec est déjà découpée en champs sur les espaces."""
    if inline:
        for entry in inline.split(","):
            entry = entry.strip()
            if entry:
                yield entry.split()
    if keys_file:
        with open(keys_file, encoding="utf-8") as fh:
            for line in fh:
                line = line.strip()
                if not line or line.startswith("#"):
                    continue
                yield line.split()


def _parse_int_field(fields: list[str], idx: int, default: int) -> int:
    """Champ entier optionnel ; absent ou `-` -> default."""
    if len(fields) > idx and fields[idx] != "-":
        return int(fields[idx])
    return default


def load_keys(
    inline: str | None,
    keys_file: str | None,
    default_rate: int,
    default_burst: int,
) -> dict[str, KeyConfig]:
    """Construit la table {clé secrète -> KeyConfig}. Format d'une spec :
    `label=CLE [rate] [burst]` (champs séparés par des espaces). `rate`/`burst`
    absents ou `-` retombent sur les défauts globaux ; un `burst` omis vaut le
    `rate` de la clé. Le label sert seulement aux logs (jamais exposé)."""
    keys: dict[str, KeyConfig] = {}
    for i, fields in enumerate(_iter_specs(inline, keys_file), 1):
        if not fields:
            continue
        labelkey = fields[0]
        if "=" in labelkey:
            label, _, secret = labelkey.partition("=")
            label, secret = label.strip(), secret.strip()
        else:
            label, secret = "", labelkey.strip()
        if not secret:
            continue
        rate = _parse_int_field(fields, 1, default_rate)
        # burst par défaut = rate de la clé (si limitée), sinon défaut global.
        burst = _parse_int_field(fields, 2, rate if rate > 0 else default_burst)
        keys.setdefault(secret, KeyConfig(label or f"key-{i}", rate, burst))
    return keys


def extract_key(headers) -> str | None:
    """Récupère la clé depuis `X-API-Key` ou `Authorization: Bearer <clé>`."""
    xkey = headers.get("X-API-Key")
    if xkey:
        return xkey.strip()
    auth = headers.get("Authorization", "")
    if auth[:7].lower() == "bearer ":
        return auth[7:].strip()
    return None


def check_key(presented: str | None) -> KeyConfig | None:
    """Renvoie la KeyConfig du client si la clé est valide, sinon None.
    Comparaison en temps constant (hmac.compare_digest) pour ne pas fuiter la
    clé au timing ; on itère sur toutes les clés même après un match."""
    # Borne la longueur avant toute comparaison : une clé présentée absurdement
    # longue (plusieurs Mo) n'a aucune chance d'être valide et éviterait sinon un
    # travail inutile (DoS CPU). 256 laisse large devant token_urlsafe(32) (~43).
    if not presented or len(presented) > 256:
        return None
    matched: KeyConfig | None = None
    for key, cfg in API_KEYS.items():
        if hmac.compare_digest(presented, key):
            matched = cfg
    return matched


# ---------------------------------------------------------------------------
# Rate-limiting : un token-bucket par clé (thread-safe)
# ---------------------------------------------------------------------------
class _Bucket:
    __slots__ = ("tokens", "updated")

    def __init__(self, tokens: float, updated: float):
        self.tokens = tokens
        self.updated = updated


class RateLimiter:
    """Token-buckets par identité (une identité = un client/label). Le débit et
    le pic sont propres à chaque clé et fournis à chaque appel (`allow`) : ainsi
    une même instance sert des clés aux limites différentes. `rate_per_min` <= 0
    => illimité. Moins d'1 jeton disponible -> refus (429) avec le délai
    d'attente. En mémoire, remis à zéro au redémarrage : suffisant pour une API
    perso mono-processus."""

    def __init__(self):
        self._buckets: dict[str, _Bucket] = {}
        self._lock = threading.Lock()

    @staticmethod
    def retry_for(tokens: float, rate_per_min: int) -> float:
        """Secondes avant que le solde repasse au seuil d'admission (1 jeton).
        0 si un jeton est déjà disponible, ou si la clé est illimitée. Source
        unique de la formule : le `Retry-After` du 429 et le `retry_after` de
        /limits doivent coïncider, sinon un client qui dort la durée annoncée
        se reprendrait un 429 au réveil."""
        rate = rate_per_min / 60.0
        if rate <= 0 or tokens >= 1.0:
            return 0.0
        return (1.0 - tokens) / rate

    def allow(self, identity: str, rate_per_min: int, burst: int,
              cost: int = 1) -> tuple[bool, float]:
        """(autorisé, retry_after_secondes) pour cette identité, à son propre
        débit `rate_per_min` (req/min) et pic `burst`. `cost` = jetons prélevés
        (un batch coûte un jeton par adressage).

        Le seau ne peut pas contenir plus de `burst` jetons ; un batch coûteux
        pourrait donc n'être jamais servable. On passe en découvert : dès qu'il
        reste 1 jeton on autorise, puis on débite le coût entier — le solde
        devient négatif et le client attend d'avoir remboursé. La dette est
        bornée par le plus gros coût acceptable (BATCH_MAX)."""
        rate = rate_per_min / 60.0               # jetons par seconde
        if rate <= 0:
            return True, 0.0
        cap = float(max(burst, 1))
        now = time.monotonic()
        with self._lock:
            b = self._buckets.get(identity)
            if b is None:
                b = _Bucket(cap, now)
                self._buckets[identity] = b
            # Recharge proportionnelle au temps écoulé, plafonnée au pic.
            b.tokens = min(cap, b.tokens + (now - b.updated) * rate)
            b.updated = now
            if b.tokens >= 1.0:
                b.tokens -= float(max(cost, 1))
                return True, 0.0
            return False, self.retry_for(b.tokens, rate_per_min)

    def peek(self, identity: str, rate_per_min: int, burst: int) -> float | None:
        """Solde de jetons de cette identité, sans rien consommer (sert /limits).
        None si la clé est illimitée (pas de seau). Le solde est négatif quand le
        client est en découvert après un gros batch."""
        rate = rate_per_min / 60.0
        if rate <= 0:
            return None
        cap = float(max(burst, 1))
        now = time.monotonic()
        with self._lock:
            b = self._buckets.get(identity)
            if b is None:
                return cap                       # jamais vu : seau plein
            return min(cap, b.tokens + (now - b.updated) * rate)


_RL: RateLimiter | None = None


# ---------------------------------------------------------------------------
# Résolution + mise en forme de la réponse simple
# ---------------------------------------------------------------------------
def _pick_primary_ap(access_points: list[dict[str, Any]], supports: bool) -> dict[str, Any] | None:
    """La PA à remonter : en priorité celle qui annonce EXTENDED-CTC-FR, sinon
    la première. Dans le contexte CTC-FR un adressage est routé vers une seule
    PA ; on reste robuste si l'annuaire en expose plusieurs."""
    if not access_points:
        return None
    if supports:
        for ap in access_points:
            if FR_CTC_PRIMARY_INVOICE in ap.get("doctypes_supported", []):
                return ap
    return access_points[0]


def sml_lookup_error(result: dict[str, Any]) -> str | None:
    """Statut SML signalant un ÉCHEC de lookup (annuaire inconsultable), par
    opposition à une absence. Seul NXDOMAIN — réponse authentique de
    l'autoritaire — signifie « non enregistré » ; tout autre statut non-OK
    (DNS_ERROR:*, NoAnswer) veut dire qu'on n'a PAS pu consulter l'annuaire.
    Le déguiser en exists:false fabriquerait des faux négatifs silencieux
    (constaté en prod le 2026-07-13 sous rafale DNS)."""
    status = (result.get("sml") or {}).get("status")
    if status in ("OK", "NXDOMAIN"):
        return None
    return f"SML lookup: {status}"


def simple_view(result: dict[str, Any]) -> dict[str, Any]:
    """Transforme la sortie de peppol_resolver.resolve() en réponse simple :
    existence, code + nom + pays de la PA, support EXTENDED-CTC-FR."""
    sml = result.get("sml") or {}
    exists = sml.get("status") == "OK" and bool(result.get("smp_url"))

    out: dict[str, Any] = {
        "participant_id": result.get("participant_id"),
        "scheme": result.get("scheme"),
        "value": result.get("value"),
        "exists": exists,
        "pa": None,
        "supports_extended_ctc_fr": None,
    }
    if not exists:
        # Non enregistré : rien de plus à dire, ce n'est pas une erreur.
        out["supports_extended_ctc_fr"] = False
        if sml.get("status") not in (None, "OK", "NXDOMAIN", "NoAnswer"):
            out["note"] = f"SML lookup: {sml.get('status')}"
        return out

    if "access_points" not in result:
        # Enregistré mais le catalogue n'a pas pu être lu (403 SMP, ap_only…) :
        # on ne peut pas conclure sur EXTENDED-CTC-FR ni identifier la PA.
        out["note"] = result.get("error") or "SMP catalogue indisponible"
        return out

    aps = result.get("access_points") or []
    supports = any(
        FR_CTC_PRIMARY_INVOICE in ap.get("doctypes_supported", []) for ap in aps
    )
    out["supports_extended_ctc_fr"] = supports

    ap = _pick_primary_ap(aps, supports)
    if ap is not None:
        out["pa"] = {
            "code": ap.get("ap_peppol_id"),
            "name": ap.get("organization"),
            "country": ap.get("country"),
        }
    if len(aps) > 1:
        out["access_point_count"] = len(aps)
    return out


def do_resolve(participant: str, test: bool) -> dict[str, Any]:
    """Parse le PID et appelle le résolveur direct (SML+SMP)."""
    scheme, value = resolver.parse_pid(participant)
    zone = resolver.SML_TEST if test else resolver.SML_PROD
    return resolver.resolve(scheme, value, zone)


def resolve_item(raw: str, test: bool) -> dict[str, Any]:
    """Résout un participant pour le batch, en isolant les erreurs (une entrée
    en échec ne casse pas le lot) et sous le sémaphore global de concurrence."""
    p = raw.strip()
    if not p:
        return {"participant": raw, "error": "Participant vide."}
    if len(p) > 255:
        return {"participant": p, "error": "Identifiant de participant trop long."}
    assert _SEM is not None
    if not _SEM.acquire(timeout=_CONCURRENCY_TIMEOUT):
        return {"participant": p, "error": "Serveur saturé."}
    try:
        try:
            result = do_resolve(p, test)
        except ValueError as e:
            return {"participant": p, "error": str(e)}
        except Exception:  # pragma: no cover - filet de sécurité
            log.exception("batch resolve failed for %r", p)
            return {"participant": p, "error": "Erreur interne."}
        # Échec de lookup SML : erreur re-tentable par entrée, jamais un
        # verdict exists:false (faux négatif silencieux).
        err = sml_lookup_error(result)
        if err:
            return {"participant": p, "error": err}
        return simple_view(result)
    finally:
        _SEM.release()


def resolve_batch(participants: list[str], test: bool) -> list[dict[str, Any]]:
    """Résout une liste de participants en parallèle (pool borné par
    MAX_CONCURRENCY) et renvoie les résultats dans l'ordre d'entrée. Les
    doublons ne sont résolus qu'une fois."""
    uniq = list(dict.fromkeys(p.strip() for p in participants))
    workers = max(1, min(len(uniq), MAX_CONCURRENCY))
    with ThreadPoolExecutor(max_workers=workers) as ex:
        resolved = dict(zip(uniq, ex.map(lambda u: resolve_item(u, test), uniq)))
    return [resolved[p.strip()] for p in participants]


# ---------------------------------------------------------------------------
# OpenAPI + Swagger UI
# ---------------------------------------------------------------------------
def openapi_spec() -> dict[str, Any]:
    """Spec OpenAPI 3.0 décrivant l'API (sert la doc Swagger sans FastAPI)."""
    return {
        "openapi": "3.0.3",
        "info": {
            "title": "Peppol Resolver API",
            "version": "1.0.0",
            "description": (
                "Résout un adressage (Participant ID Peppol) via SML+SMP et "
                "renvoie l'existence, la PA (code + nom) et le support du "
                "format UBL EXTENDED-CTC-FR."
            ),
        },
        "components": {
            "securitySchemes": {
                "ApiKeyHeader": {"type": "apiKey", "in": "header", "name": "X-API-Key"},
                "BearerAuth": {"type": "http", "scheme": "bearer"},
            },
            "schemas": {
                "ResolveResponse": {
                    "type": "object",
                    "properties": {
                        "participant_id": {"type": "string", "example": "iso6523-actorid-upis::0225:000122308"},
                        "scheme": {"type": "string", "example": "iso6523-actorid-upis"},
                        "value": {"type": "string", "example": "0225:000122308"},
                        "exists": {"type": "boolean"},
                        "supports_extended_ctc_fr": {"type": "boolean", "nullable": True},
                        "pa": {
                            "type": "object",
                            "nullable": True,
                            "properties": {
                                "code": {"type": "string", "example": "PFR000123"},
                                "name": {"type": "string", "example": "Exemple SAS"},
                                "country": {"type": "string", "example": "FR"},
                            },
                        },
                        "access_point_count": {"type": "integer"},
                        "note": {"type": "string"},
                    },
                },
                "Error": {
                    "type": "object",
                    "properties": {"error": {"type": "string"}},
                },
                "BatchRequest": {
                    "type": "object",
                    "required": ["participants"],
                    "properties": {
                        "participants": {
                            "type": "array",
                            "items": {"type": "string"},
                            "maxItems": BATCH_MAX,
                            "example": ["0225:000122308", "iso6523-actorid-upis::0225:931153688"],
                        },
                        "test": {"type": "boolean", "description": "SML de test (SMK) pour tout le lot."},
                    },
                },
                "BatchResponse": {
                    "type": "object",
                    "properties": {
                        "count": {"type": "integer"},
                        "results": {
                            "type": "array",
                            "description": "Un élément par participant, dans l'ordre. Chaque "
                                           "élément est une ResolveResponse, ou {participant, error}.",
                            "items": {"$ref": "#/components/schemas/ResolveResponse"},
                        },
                    },
                },
            },
        },
        "security": [{"ApiKeyHeader": []}, {"BearerAuth": []}],
        "paths": {
            "/health": {
                "get": {
                    "summary": "Liveness (public, sans clé)",
                    "security": [],
                    "responses": {"200": {"description": "OK"}},
                }
            },
            "/resolve/{participant}": {
                "get": {
                    "summary": "Résout un adressage Peppol",
                    "parameters": [
                        {
                            "name": "participant", "in": "path", "required": True,
                            "schema": {"type": "string"},
                            "description": "Ex. '0225:000122308' ou 'iso6523-actorid-upis::0225:000122308'.",
                        },
                        {
                            "name": "test", "in": "query", "required": False,
                            "schema": {"type": "boolean"},
                            "description": "Interroge le SML de test (SMK).",
                        },
                        {
                            "name": "detail", "in": "query", "required": False,
                            "schema": {"type": "string", "enum": ["full"]},
                            "description": "'full' ajoute le JSON complet du résolveur (champ 'detail').",
                        },
                    ],
                    "responses": {
                        "200": {
                            "description": "Résultat",
                            "content": {"application/json": {"schema": {"$ref": "#/components/schemas/ResolveResponse"}}},
                        },
                        "400": {"description": "Participant invalide"},
                        "401": {"description": "Clé d'API manquante ou invalide"},
                        "429": {"description": "Rate limit dépassé"},
                        "503": {"description": "Serveur saturé"},
                    },
                }
            },
            "/limits": {
                "get": {
                    "summary": "Quota de rate-limit de la clé présentée",
                    "description": (
                        "Ne prélève aucun jeton : consulter son solde ne le réduit "
                        "pas. 'current' est le nombre de jetons restants — négatif "
                        "si le client est en découvert après un gros batch. "
                        "'retry_after' donne les secondes à attendre avant de pouvoir "
                        "rappeler (0 si un jeton est disponible) ; c'est la même valeur "
                        "que le 'Retry-After' d'un 429. "
                        "'limit' = 0 signifie illimité, et 'current' vaut alors null."
                    ),
                    "security": [{"ApiKeyHeader": []}, {"BearerAuth": []}],
                    "responses": {
                        "200": {
                            "description": "Quota",
                            "content": {"application/json": {"schema": {
                                "type": "object",
                                "properties": {
                                    "current": {"type": "number", "nullable": True,
                                                "description": "Jetons restants (< 0 = découvert)."},
                                    "limit": {"type": "integer",
                                              "description": "Débit en requêtes/min (0 = illimité)."},
                                    "burst": {"type": "integer",
                                              "description": "Capacité maximale du seau."},
                                    "retry_after": {"type": "number",
                                                    "description": "Secondes à attendre avant de "
                                                                   "rappeler (0 si un jeton est dispo)."},
                                },
                            }}},
                        },
                        "401": {"description": "Clé d'API manquante ou invalide"},
                    },
                }
            },
            "/resolve/batch": {
                "post": {
                    "summary": "Résout plusieurs adressages en une requête",
                    "description": (
                        f"Jusqu'à {BATCH_MAX} participants. Résolution en parallèle "
                        "(pool borné). Une entrée en échec n'échoue pas le lot : "
                        "elle porte alors un champ 'error'. Compte 1 jeton de "
                        "rate-limit par participant : le solde peut passer en "
                        "négatif, la requête suivante attend qu'il repasse à 1."
                    ),
                    "requestBody": {
                        "required": True,
                        "content": {"application/json": {
                            "schema": {"$ref": "#/components/schemas/BatchRequest"}}},
                    },
                    "responses": {
                        "200": {
                            "description": "Résultats (dans l'ordre d'entrée)",
                            "content": {"application/json": {
                                "schema": {"$ref": "#/components/schemas/BatchResponse"}}},
                        },
                        "400": {"description": "Corps invalide ou batch trop grand"},
                        "401": {"description": "Clé d'API manquante ou invalide"},
                        "429": {"description": "Rate limit dépassé"},
                    },
                }
            },
        },
    }


_SWAGGER_HTML = """<!doctype html>
<html lang="fr">
<head>
  <meta charset="utf-8"/>
  <meta name="viewport" content="width=device-width, initial-scale=1"/>
  <title>Peppol Resolver API — doc</title>
  <link rel="stylesheet" href="https://unpkg.com/swagger-ui-dist@5/swagger-ui.css"/>
</head>
<body>
  <div id="swagger-ui"></div>
  <script src="https://unpkg.com/swagger-ui-dist@5/swagger-ui-bundle.js" crossorigin></script>
  <script>
    window.ui = SwaggerUIBundle({
      url: "openapi.json",
      dom_id: "#swagger-ui",
      deepLinking: true,
    });
  </script>
</body>
</html>
"""


# ---------------------------------------------------------------------------
# Handler HTTP
# ---------------------------------------------------------------------------
class Handler(BaseHTTPRequestHandler):
    server_version = "peppol-api/1.0"
    protocol_version = "HTTP/1.1"

    # --- helpers réponse ---------------------------------------------------
    def _send_json(self, code: int, payload: dict[str, Any], extra_headers: dict[str, str] | None = None) -> None:
        body = json.dumps(payload, ensure_ascii=False, default=str).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        for k, v in (extra_headers or {}).items():
            self.send_header(k, v)
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(body)

    def _send_text(self, code: int, text: str, content_type: str = "text/plain; charset=utf-8") -> None:
        body = text.encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(body)

    def _error(self, code: int, message: str, extra_headers: dict[str, str] | None = None) -> None:
        self._send_json(code, {"error": message}, extra_headers)

    # --- logging -----------------------------------------------------------
    def _client_ip(self) -> str:
        """IP réelle du client. Le serveur n'écoutant qu'en local derrière nginx,
        address_string() vaut toujours 127.0.0.1 ; on préfère l'IP transmise par
        le proxy (X-Real-IP / 1er X-Forwarded-For) quand elle est présente."""
        headers = getattr(self, "headers", None)
        if headers:
            for h in ("X-Real-IP", "X-Forwarded-For"):
                val = headers.get(h)
                if val:
                    return val.split(",")[0].strip()
        return self.address_string()

    def log_message(self, fmt: str, *args: Any) -> None:  # noqa: A003
        log.info("%s - %s", self._client_ip(), fmt % args)

    # --- routing -----------------------------------------------------------
    def do_GET(self) -> None:
        self._route()

    def do_HEAD(self) -> None:
        self._route()

    def do_POST(self) -> None:
        parsed = urllib.parse.urlparse(self.path)
        path = parsed.path.rstrip("/") or "/"
        if path == "/resolve/batch":
            return self._handle_batch()
        return self._error(HTTPStatus.NOT_FOUND, "Ressource inconnue")

    def _route(self) -> None:
        parsed = urllib.parse.urlparse(self.path)
        path = parsed.path.rstrip("/") or "/"

        # Endpoints publics (pas de clé).
        if path == "/health":
            return self._send_json(HTTPStatus.OK, {"status": "ok"})
        if path == "/openapi.json":
            return self._send_json(HTTPStatus.OK, openapi_spec())
        if path in ("/docs", "/"):
            return self._send_text(HTTPStatus.OK, _SWAGGER_HTML, "text/html; charset=utf-8")

        # /resolve/batch n'accepte que POST.
        if path == "/resolve/batch":
            return self._error(
                HTTPStatus.METHOD_NOT_ALLOWED,
                "Utilisez POST sur /resolve/batch.", {"Allow": "POST"},
            )

        # À partir d'ici : clé d'API requise.
        if path == "/limits":
            return self._handle_limits()
        if path == "/resolve" or path.startswith("/resolve/"):
            return self._handle_resolve(parsed, path)

        return self._error(HTTPStatus.NOT_FOUND, "Ressource inconnue")

    # --- /limits -----------------------------------------------------------
    def _handle_limits(self) -> None:
        """Limites de la clé présentée. Ne prélève aucun jeton : consulter son
        solde ne doit pas le réduire. `current` est négatif quand le client est
        en découvert (gros batch récent) ; `limit` = 0 signifie illimité, et
        `current` vaut alors null (pas de seau)."""
        cfg = check_key(extract_key(self.headers))
        if cfg is None:
            return self._error(
                HTTPStatus.UNAUTHORIZED,
                "Clé d'API manquante ou invalide (header X-API-Key ou Authorization: Bearer).",
                {"WWW-Authenticate": "Bearer"},
            )
        assert _RL is not None
        current = _RL.peek(cfg.label, cfg.rate, cfg.burst)
        retry = 0.0 if current is None else _RL.retry_for(current, cfg.rate)
        return self._send_json(HTTPStatus.OK, {
            "current": round(current, 2) if current is not None else None,
            "limit": cfg.rate,
            "burst": cfg.burst,
            "retry_after": round(retry, 2),
        })

    # --- /resolve ----------------------------------------------------------
    def _handle_resolve(self, parsed: urllib.parse.ParseResult, path: str) -> None:
        cfg = check_key(extract_key(self.headers))
        if cfg is None:
            return self._error(
                HTTPStatus.UNAUTHORIZED,
                "Clé d'API manquante ou invalide (header X-API-Key ou Authorization: Bearer).",
                {"WWW-Authenticate": "Bearer"},
            )

        # Rate-limit propre à cette clé (repli sur le défaut global au chargement).
        assert _RL is not None
        allowed, retry = _RL.allow(cfg.label, cfg.rate, cfg.burst)
        if not allowed:
            return self._error(
                HTTPStatus.TOO_MANY_REQUESTS,
                "Rate limit dépassé, réessayez plus tard.",
                {"Retry-After": str(int(retry) + 1)},
            )

        qs = urllib.parse.parse_qs(parsed.query)
        if path.startswith("/resolve/"):
            participant = urllib.parse.unquote(path[len("/resolve/"):])
        else:
            participant = (qs.get("participant") or [""])[0]
        participant = participant.strip()
        if not participant:
            return self._error(HTTPStatus.BAD_REQUEST, "Paramètre 'participant' manquant.")
        # Garde-fou : un Participant ID Peppol tient largement en 255 caractères ;
        # au-delà c'est une entrée anormale, inutile de la passer au résolveur.
        if len(participant) > 255:
            return self._error(HTTPStatus.BAD_REQUEST, "Identifiant de participant trop long.")

        test = _truthy((qs.get("test") or ["0"])[0])
        want_detail = (qs.get("detail") or [""])[0] == "full"

        # Concurrence bornée : les résolutions sont I/O-bound (réseau Peppol).
        assert _SEM is not None
        if not _SEM.acquire(timeout=_CONCURRENCY_TIMEOUT):
            return self._error(HTTPStatus.SERVICE_UNAVAILABLE, "Serveur saturé, réessayez.")
        try:
            try:
                result = do_resolve(participant, test)
            except ValueError as e:
                return self._error(HTTPStatus.BAD_REQUEST, str(e))
            except Exception:  # pragma: no cover - filet de sécurité
                # Détail journalisé côté serveur ; on ne renvoie rien de sensible
                # (chemins, versions…) au client.
                log.exception("resolve failed for %r", participant)
                return self._error(HTTPStatus.INTERNAL_SERVER_ERROR, "Erreur interne.")
        finally:
            _SEM.release()

        # Échec de lookup SML : l'annuaire n'a pas pu être consulté — 503
        # re-tentable, jamais un 200 exists:false (faux négatif silencieux).
        err = sml_lookup_error(result)
        if err:
            return self._error(
                HTTPStatus.SERVICE_UNAVAILABLE,
                f"Annuaire Peppol momentanément inaccessible ({err}). Réessayez.",
            )

        payload = simple_view(result)
        if want_detail:
            payload["detail"] = result
        return self._send_json(HTTPStatus.OK, payload)

    # --- /resolve/batch ----------------------------------------------------
    def _read_json_body(self, max_bytes: int) -> tuple[Any, str | None]:
        """Lit et parse le corps JSON. Renvoie (data, erreur). En cas de corps
        absent/trop gros, coupe la connexion (Connection: close) pour éviter un
        désync keep-alive HTTP/1.1 (corps non consommé)."""
        length = self.headers.get("Content-Length")
        if length is None:
            self.close_connection = True
            return None, "Content-Length requis."
        try:
            n = int(length)
        except ValueError:
            self.close_connection = True
            return None, "Content-Length invalide."
        if n < 0 or n > max_bytes:
            self.close_connection = True
            return None, f"Corps trop volumineux (max {max_bytes} octets)."
        raw = self.rfile.read(n) if n else b""
        if not raw:
            return None, "Corps JSON vide."
        try:
            return json.loads(raw.decode("utf-8")), None
        except Exception:
            return None, "Corps JSON invalide."

    def _handle_batch(self) -> None:
        # On lit le corps d'abord (keep-alive sûr), puis on authentifie.
        body, body_err = self._read_json_body(BATCH_BODY_MAX_BYTES)

        cfg = check_key(extract_key(self.headers))
        if cfg is None:
            return self._error(
                HTTPStatus.UNAUTHORIZED,
                "Clé d'API manquante ou invalide (header X-API-Key ou Authorization: Bearer).",
                {"WWW-Authenticate": "Bearer"},
            )
        # Validation avant rate-limit : une requête malformée est gratuite (elle
        # ne consomme aucun jeton), et il faut de toute façon connaître le nombre
        # d'adressages pour en calculer le coût.
        if body_err:
            return self._error(HTTPStatus.BAD_REQUEST, body_err)
        if not isinstance(body, dict):
            return self._error(HTTPStatus.BAD_REQUEST,
                               'Corps attendu : objet {"participants": [...]}.')
        participants = body.get("participants")
        if not isinstance(participants, list) or not participants:
            return self._error(HTTPStatus.BAD_REQUEST,
                               "Champ 'participants' (liste non vide) requis.")
        if len(participants) > BATCH_MAX:
            return self._error(
                HTTPStatus.BAD_REQUEST,
                f"Batch trop grand : {len(participants)} participants > {BATCH_MAX} max.",
            )
        if not all(isinstance(p, str) for p in participants):
            return self._error(HTTPStatus.BAD_REQUEST,
                               "'participants' doit être une liste de chaînes.")
        test = bool(body.get("test", False))

        # Rate-limit : un jeton par adressage — un batch coûte ce qu'il coûte
        # vraiment au résolveur, et non le prix d'une requête unitaire.
        assert _RL is not None
        allowed, retry = _RL.allow(cfg.label, cfg.rate, cfg.burst,
                                   cost=len(participants))
        if not allowed:
            return self._error(
                HTTPStatus.TOO_MANY_REQUESTS,
                "Rate limit dépassé, réessayez plus tard.",
                {"Retry-After": str(int(retry) + 1)},
            )

        results = resolve_batch(participants, test)
        return self._send_json(HTTPStatus.OK, {"count": len(results), "results": results})


def _truthy(v: str) -> bool:
    return v.strip().lower() in ("1", "true", "yes", "on")


# ---------------------------------------------------------------------------
# Démarrage
# ---------------------------------------------------------------------------
class _Server(ThreadingHTTPServer):
    daemon_threads = True
    allow_reuse_address = True


def build_config(args: argparse.Namespace) -> None:
    """Charge clés, rate-limiter, sémaphore de concurrence et configure le
    réseau du résolveur. Renseigne les globals utilisés par le handler."""
    global API_KEYS, DEFAULT_RATE_PER_MIN, DEFAULT_BURST, MAX_CONCURRENCY, _RL, _SEM

    DEFAULT_RATE_PER_MIN = args.rate_limit
    DEFAULT_BURST = args.rate_burst if args.rate_burst is not None else args.rate_limit
    MAX_CONCURRENCY = max(1, args.max_concurrency)

    API_KEYS = load_keys(
        args.keys if args.keys is not None else os.environ.get("PEPPOL_API_KEYS"),
        args.keys_file if args.keys_file is not None else os.environ.get("PEPPOL_API_KEYS_FILE"),
        DEFAULT_RATE_PER_MIN,
        DEFAULT_BURST,
    )
    if not API_KEYS:
        raise SystemExit(
            "ERREUR : aucune clé d'API définie. Renseignez --keys / --keys-file "
            "ou PEPPOL_API_KEYS / PEPPOL_API_KEYS_FILE. "
            "Générez-en une avec: python peppol_api.py --gen-key"
        )

    _RL = RateLimiter()
    _SEM = threading.BoundedSemaphore(MAX_CONCURRENCY)

    resolver.configure_network(
        proxy=args.proxy,
        ca_bundle=args.ca_bundle,
        insecure=args.insecure,
        dns_server=args.dns_server,
        doh_endpoint=args.doh,
        dns_fallback=args.dns_fallback or None,
    )


def _env_int(name: str, default: int) -> int:
    v = os.environ.get(name)
    try:
        return int(v) if v is not None else default
    except ValueError:
        return default


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--host", default=os.environ.get("PEPPOL_API_HOST", "127.0.0.1"),
                    help="Interface d'écoute (défaut 127.0.0.1, à laisser derrière nginx).")
    ap.add_argument("--port", type=int, default=_env_int("PEPPOL_API_PORT", 8080),
                    help="Port d'écoute (défaut 8080).")
    ap.add_argument("--keys", default=None,
                    help="Clés séparées par des virgules ; chaque entrée "
                         "'label=CLE [rate] [burst]' (sinon PEPPOL_API_KEYS).")
    ap.add_argument("--keys-file", default=None,
                    help="Fichier de clés (une spec/ligne 'label=CLE [rate] [burst]', "
                         "'#' commentaire).")
    ap.add_argument("--rate-limit", type=int, default=_env_int("PEPPOL_API_RATE_LIMIT", 60),
                    help="Requêtes/min par DÉFAUT, pour les clés sans rate propre "
                         "(0 = illimité, défaut 60).")
    ap.add_argument("--rate-burst", type=int, default=_env_int("PEPPOL_API_RATE_BURST", None) or None,
                    help="Pic par défaut (défaut = rate-limit).")
    ap.add_argument("--max-concurrency", type=int,
                    default=_env_int("PEPPOL_API_MAX_CONCURRENCY", MAX_CONCURRENCY),
                    help="Résolutions simultanées max (défaut 8).")
    # Passe-plats réseau vers peppol_resolver.configure_network().
    ap.add_argument("--proxy", default=None, help="Proxy HTTP(S) (sinon HTTP_PROXY/HTTPS_PROXY).")
    ap.add_argument("--ca-bundle", default=None, help="CA bundle (interception TLS d'entreprise).")
    ap.add_argument("--insecure", action="store_true", help="Désactive la vérif TLS (à éviter).")
    ap.add_argument("--dns-server", default=os.environ.get("PEPPOL_API_DNS_SERVER"),
                    help="Serveur DNS pour les NAPTR, ex. 1.1.1.1 "
                         "(sinon PEPPOL_API_DNS_SERVER, sinon /etc/resolv.conf).")
    ap.add_argument("--doh", nargs="?", const=resolver.DEFAULT_DOH_ENDPOINT,
                    default=os.environ.get("PEPPOL_API_DOH"),
                    metavar="ENDPOINT", help="Active DNS-over-HTTPS (sinon PEPPOL_API_DOH).")
    ap.add_argument("--dns-fallback",
                    default=os.environ.get("PEPPOL_API_DNS_FALLBACK", "8.8.8.8"),
                    help="Résolveur de secours consulté quand le principal échoue "
                         "après retries (cache négatif empoisonné). Défaut 8.8.8.8 ; "
                         "chaîne vide pour désactiver (sinon PEPPOL_API_DNS_FALLBACK).")
    ap.add_argument("--gen-key", action="store_true",
                    help="Génère une clé d'API sûre et quitte.")
    args = ap.parse_args()

    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s %(levelname)s %(name)s: %(message)s",
    )

    if args.gen_key:
        print(secrets.token_urlsafe(32))
        return 0

    build_config(args)

    httpd = _Server((args.host, args.port), Handler)
    n_custom = sum(1 for c in API_KEYS.values() if c.rate != DEFAULT_RATE_PER_MIN)
    log.info(
        "Peppol Resolver API sur http://%s:%d — %d clé(s) (%d à rate custom), "
        "rate-limit défaut %s req/min, concurrence %d. Doc: /docs",
        args.host, args.port, len(API_KEYS), n_custom,
        DEFAULT_RATE_PER_MIN or "∞", max(1, args.max_concurrency),
    )
    try:
        httpd.serve_forever()
    except KeyboardInterrupt:
        log.info("Arrêt.")
    finally:
        httpd.server_close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
