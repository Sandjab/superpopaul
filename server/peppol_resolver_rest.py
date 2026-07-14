#!/usr/bin/env python3
"""
Peppol Participant Resolver (variante REST) — du scheme:value à la fiche
complète des Access Points, via l'API REST publique de Helger plutôt qu'en
attaquant SML+SMP directement.

    https://peppol.helger.com/public/locale-en_US/menuitem-tools-rest-api

Même sortie que peppol_resolver.py, mais tout le pipeline (résolution SML,
lookup DNS NAPTR, requêtes SMP, parsing XML, parsing du certificat X.509) est
fait côté serveur par Helger et renvoyé en JSON. Correspondance :

    peppol_resolver.py (SML+SMP direct)     ->  ici (API Helger, JSON)
    ----------------------------------------------------------------------
    sml_hostname + NAPTR DNS -> smp_url      ->  GET /ppidexistence/{sml}/{pid}
    fetch_service_group (ServiceGroup XML)   ->  GET /smpquery/{sml}/{pid}
    fetch_service_metadata + parse_cert      ->  GET /smpquery/{sml}/{pid}/{docType}

Différences assumées par rapport à la version directe :
  * Plus aucune dépendance (dnspython et cryptography ne sont plus nécessaires,
    ni le parsing XML) : stdlib pure.
  * fingerprint_sha256 : non fourni par l'API, recalculé en SHA-256 du DER
    (base64 -> hashlib) — identique à ce que produisait cryptography.
  * service_activation_date / service_expiration_date : non exposés par cette
    API REST, restent None.
  * Options DNS retirées (--dns-server, --doh) : plus de résolution DNS côté
    client. Proxy / CA bundle / insecure / retry 429 conservés.

Usage :
    python peppol_resolver_rest.py 0225:000122308
    python peppol_resolver_rest.py iso6523-actorid-upis::0225:000122308
    python peppol_resolver_rest.py 0225:000122308 --full      # JSON brut complet
    python peppol_resolver_rest.py 0225:000122308 --test      # SML de test (SMK)
    python peppol_resolver_rest.py 0225:000122308 --ap-only   # s'arrête au SMP
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import random
import ssl
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from typing import Any
from urllib.parse import quote

# API REST publique de Helger. smpquery/businesscard/ppidexistence attaquent la
# production ("peppolprod") ou le réseau de test ("peppoltest").
HELGER_API_BASE = "https://peppol.helger.com/api"
SML_PROD = "peppolprod"
SML_TEST = "peppoltest"
DEFAULT_SCHEME = "iso6523-actorid-upis"

HTTP_TIMEOUT = 20
UA = "peppol-resolver/1.0 (+research)"

# Retry avec backoff sur les statuts transitoires (l'API renvoie 429 quand on la
# sollicite trop vite). Retry-After respecté.
HTTP_RETRY_STATUSES = frozenset({429, 503})
HTTP_MAX_RETRIES = 4
HTTP_RETRY_BASE = 1.0          # secondes (×2 à chaque tentative + jitter)

_DEBUG = False


def _dbg(msg: str) -> None:
    if _DEBUG:
        print(f"[debug] {msg}", file=sys.stderr)


# Initialized by configure_network(); fall back to defaults if never called.
_HTTP_OPENER: urllib.request.OpenerDirector | None = None


def _resolve_ca_bundle(explicit: str | None) -> str | None:
    """Pick a CA bundle path from CLI arg or common env vars."""
    if explicit:
        return explicit
    for var in ("SSL_CERT_FILE", "REQUESTS_CA_BUNDLE", "CURL_CA_BUNDLE"):
        v = os.environ.get(var)
        if v:
            return v
    return None


def configure_network(
    proxy: str | None = None,
    ca_bundle: str | None = None,
    insecure: bool = False,
) -> None:
    """Build the global HTTP opener used to reach the Helger API.

    Proxy: explicit URL overrides env (HTTP_PROXY/HTTPS_PROXY/NO_PROXY). If both
    are unset, urllib uses a direct connection.

    CA bundle: explicit path overrides SSL_CERT_FILE/REQUESTS_CA_BUNDLE/
    CURL_CA_BUNDLE. Required when a corporate proxy does TLS interception.
    """
    global _HTTP_OPENER

    handlers: list[urllib.request.BaseHandler] = []

    if proxy:
        handlers.append(urllib.request.ProxyHandler({"http": proxy, "https": proxy}))
    else:
        handlers.append(urllib.request.ProxyHandler())  # picks up env vars

    ctx = ssl.create_default_context()
    bundle = _resolve_ca_bundle(ca_bundle)
    if bundle:
        ctx.load_verify_locations(cafile=bundle)
    if insecure:
        ctx.check_hostname = False
        ctx.verify_mode = ssl.CERT_NONE
    handlers.append(urllib.request.HTTPSHandler(context=ctx))

    _HTTP_OPENER = urllib.request.build_opener(*handlers)


def _opener() -> urllib.request.OpenerDirector:
    if _HTTP_OPENER is None:
        configure_network()
    assert _HTTP_OPENER is not None
    return _HTTP_OPENER


def parse_pid(raw: str) -> tuple[str, str]:
    """Accept '0225:value', 'iso6523-actorid-upis::0225:value', etc."""
    raw = raw.strip()
    if "::" in raw:
        scheme, value = raw.split("::", 1)
    else:
        scheme, value = DEFAULT_SCHEME, raw
    if not scheme or not value:
        raise ValueError(f"Cannot parse participant identifier: {raw!r}")
    return scheme, value


def _retry_after_seconds(e: urllib.error.HTTPError, attempt: int) -> float:
    """Délai avant retry : en-tête Retry-After si présent, sinon backoff
    exponentiel + jitter."""
    ra = e.headers.get("Retry-After") if e.headers else None
    if ra:
        try:
            return max(0.0, float(ra))            # Retry-After en secondes
        except ValueError:
            pass                                   # format date HTTP : ignoré
    return HTTP_RETRY_BASE * (2 ** attempt) + random.uniform(0, 0.5)


def http_get(url: str, accept: str = "application/json", ua: str = UA) -> str:
    req = urllib.request.Request(url, headers={"Accept": accept, "User-Agent": ua})
    _dbg(f"GET {url}")
    _dbg(f"  > Accept: {accept}")
    for attempt in range(HTTP_MAX_RETRIES + 1):
        try:
            with _opener().open(req, timeout=HTTP_TIMEOUT) as resp:
                body = resp.read().decode("utf-8", errors="replace")
                _dbg(f"  < {resp.status} {resp.reason}")
                _dbg(f"  < body[:300] = {body[:300]!r}")
                return body
        except urllib.error.HTTPError as e:
            # Lire le corps une seule fois et l'attacher : certains endpoints
            # (ppidexistence) renvoient un JSON exploitable même en 404, et le
            # body d'une HTTPError n'est lisible qu'une fois.
            try:
                setattr(e, "_body_text", e.read().decode("utf-8", errors="replace"))
            except Exception:
                setattr(e, "_body_text", "")
            _dbg(f"  < HTTP {e.code} {e.reason}")
            _dbg(f"  < body[:300] = {e._body_text[:300]!r}")
            if e.code in HTTP_RETRY_STATUSES and attempt < HTTP_MAX_RETRIES:
                delay = _retry_after_seconds(e, attempt)
                _dbg(f"  ↻ retry {attempt + 1}/{HTTP_MAX_RETRIES} dans {delay:.1f}s")
                time.sleep(delay)
                continue
            raise
    raise RuntimeError("unreachable")  # keeps type-checkers happy


def http_get_json(url: str) -> Any:
    return json.loads(http_get(url, accept="application/json"))


def _existence_body(e: urllib.error.HTTPError) -> dict[str, Any] | None:
    """Corps JSON d'une réponse ppidexistence, y compris en 404 : l'API répond
    404 AVEC {"exists": false, ...} pour un participant simplement absent.
    Renvoie None si le corps n'est pas ce JSON-là (vraie erreur : 429, 5xx…)."""
    body = getattr(e, "_body_text", None)
    if body is None:
        try:
            body = e.read().decode("utf-8", errors="replace")
        except Exception:
            return None
    try:
        data = json.loads(body)
    except Exception:
        return None
    return data if isinstance(data, dict) and "exists" in data else None


def query_existence(sml_id: str, pid_full: str) -> dict[str, Any]:
    """GET /ppidexistence — remplace le lookup SML+NAPTR : donne l'URI du SMP
    et si le participant est enregistré. Un participant absent se traduit par
    exists=False (l'API le signale par un 404 dont le corps porte "exists")."""
    url = f"{HELGER_API_BASE}/ppidexistence/{sml_id}/{quote(pid_full, safe='')}"
    out: dict[str, Any] = {"status": None, "exists": None, "smp_host_uri": None}
    try:
        data = http_get_json(url)
    except urllib.error.HTTPError as e:
        data = _existence_body(e)
        if data is None:
            out["status"] = f"HTTP {e.code}"       # vraie erreur, on la remonte
            return out
    except Exception as e:
        out["status"] = f"{type(e).__name__}: {e}"
        return out
    out["status"] = "OK"
    out["exists"] = bool(data.get("exists"))
    out["smp_host_uri"] = data.get("smpHostURI")
    return out


def query_doctypes(sml_id: str, pid_full: str) -> list[dict[str, Any]]:
    """GET /smpquery — remplace le ServiceGroup : liste des document types."""
    url = f"{HELGER_API_BASE}/smpquery/{sml_id}/{quote(pid_full, safe='')}"
    data = http_get_json(url)
    return data.get("urls", []) or []


def _cert_from_details(cert_b64: str | None, details: dict[str, Any]) -> dict[str, Any]:
    """Traduit certificateDetails (déjà parsé par Helger) vers le même dict que
    parse_cert() de la version directe. Le fingerprint SHA-256, absent du JSON,
    est recalculé sur le DER (base64 -> hashlib) : identique à cryptography."""
    subj = details.get("subject") or {}
    iss = details.get("issuer") or {}
    fingerprint = None
    if cert_b64:
        try:
            fingerprint = hashlib.sha256(base64.b64decode(cert_b64)).hexdigest()
        except Exception:
            fingerprint = None
    serial = details.get("serial10")
    return {
        "subject_common_name": subj.get("CN"),
        "subject_organization": subj.get("O"),
        "subject_organizational_unit": subj.get("OU"),
        "subject_country": subj.get("C"),
        "issuer_common_name": iss.get("CN"),
        "issuer_organization": iss.get("O"),
        "serial_number": str(serial) if serial is not None else details.get("serial16"),
        "not_valid_before": details.get("notBefore"),
        "not_valid_after": details.get("notAfter"),
        "fingerprint_sha256": fingerprint,
    }


def query_endpoints(sml_id: str, pid_full: str, doctype_id: str) -> list[dict[str, Any]]:
    """GET /smpquery/{sml}/{pid}/{docType} — remplace ServiceMetadata : renvoie
    la liste des endpoints (un par Process×Endpoint) avec le certificat parsé."""
    url = (
        f"{HELGER_API_BASE}/smpquery/{sml_id}"
        f"/{quote(pid_full, safe='')}/{quote(doctype_id, safe='')}"
    )
    data = http_get_json(url)

    # documentTypeID = "<scheme>::<value>" ; on scinde comme la version XML
    # (scheme dans l'attribut, valeur dans le texte).
    doc_scheme, _, doc_value = (data.get("documentTypeID") or "").partition("::")
    if not doc_value:
        doc_scheme, doc_value = None, data.get("documentTypeID")

    endpoints: list[dict[str, Any]] = []
    serviceinfo = data.get("serviceinfo") or {}
    for proc in serviceinfo.get("processes", []) or []:
        process_id = proc.get("processID")
        for ep in proc.get("endpoints", []) or []:
            details = ep.get("certificateDetails") or {}
            cert = _cert_from_details(ep.get("certificate"), details) if details else None
            endpoints.append({
                "document_identifier": doc_value,
                "document_scheme": doc_scheme,
                "process_identifier": process_id,
                "transport_profile": ep.get("transportProfile"),
                "endpoint_url": ep.get("endpointReference"),
                "require_business_signature": ep.get("requireBusinessLevelSignature"),
                "service_activation_date": None,   # non exposé par l'API REST
                "service_expiration_date": None,   # non exposé par l'API REST
                "service_description": ep.get("serviceDescription"),
                "technical_contact_url": ep.get("technicalContactUrl"),
                "certificate": cert,
            })
    return endpoints


def resolve(
    scheme: str,
    value: str,
    sml_id: str = SML_PROD,
    ap_only: bool = False,
) -> dict[str, Any]:
    pid_full = f"{scheme}::{value}"
    result: dict[str, Any] = {
        "participant_id": pid_full,
        "scheme": scheme,
        "value": value,
        "sml_id": sml_id,
    }

    existence = query_existence(sml_id, pid_full)
    result["existence"] = existence
    if existence["status"] != "OK" or not existence["exists"]:
        return result

    smp_url = existence["smp_host_uri"]
    result["smp_url"] = smp_url
    result["smp_hostname"] = urllib.parse.urlparse(smp_url).hostname if smp_url else None

    if ap_only:
        return result

    try:
        doctypes = query_doctypes(sml_id, pid_full)
    except urllib.error.HTTPError as e:
        result["error"] = f"smpquery HTTP {e.code}"
        return result
    except Exception as e:
        result["error"] = f"smpquery: {type(e).__name__}: {e}"
        return result

    result["service_metadata_refs_count"] = len(doctypes)

    all_endpoints: list[dict[str, Any]] = []
    for dt in doctypes:
        doctype_id = dt.get("documentTypeID")
        if not doctype_id:
            continue
        try:
            all_endpoints.extend(query_endpoints(sml_id, pid_full, doctype_id))
        except Exception as e:
            all_endpoints.append({"_fetch_error": f"{doctype_id}: {type(e).__name__}"})

    result["endpoints"] = all_endpoints

    # Synthèse : agrège par Access Point (clé = subject_common_name du cert).
    aps: dict[str, dict[str, Any]] = {}
    for ep in all_endpoints:
        cert = ep.get("certificate")
        if not cert or "error" in cert:
            continue
        ap_id = cert.get("subject_common_name") or "<unknown>"
        ap = aps.setdefault(ap_id, {
            "ap_peppol_id": ap_id,
            "organization": cert.get("subject_organization"),
            "organizational_unit": cert.get("subject_organizational_unit"),
            "country": cert.get("subject_country"),
            "issuer_ca": cert.get("issuer_common_name"),
            "issuer_organization": cert.get("issuer_organization"),
            "cert_valid_from": cert.get("not_valid_before"),
            "cert_valid_to": cert.get("not_valid_after"),
            "cert_serial": cert.get("serial_number"),
            "cert_fingerprint_sha256": cert.get("fingerprint_sha256"),
            "endpoint_urls": set(),
            "transport_profiles": set(),
            "doctypes_supported": set(),
            "process_identifiers": set(),
            "technical_contact_urls": set(),
        })
        if ep.get("endpoint_url"):
            ap["endpoint_urls"].add(ep["endpoint_url"])
        if ep.get("transport_profile"):
            ap["transport_profiles"].add(ep["transport_profile"])
        if ep.get("document_identifier"):
            ap["doctypes_supported"].add(ep["document_identifier"])
        if ep.get("process_identifier"):
            ap["process_identifiers"].add(ep["process_identifier"])
        if ep.get("technical_contact_url"):
            ap["technical_contact_urls"].add(ep["technical_contact_url"])

    for ap in aps.values():
        for k in ("endpoint_urls", "transport_profiles", "doctypes_supported",
                  "process_identifiers", "technical_contact_urls"):
            ap[k] = sorted(ap[k])

    result["access_points"] = list(aps.values())
    return result


def print_summary(r: dict[str, Any]) -> None:
    print(f"Participant ID  : {r['participant_id']}")
    print(f"SML-ID          : {r.get('sml_id', '?')}")
    ex = r.get("existence", {})
    print(f"Existence lookup: {ex.get('status', '?')}")
    print(f"Registered      : {ex.get('exists')}")
    if r.get("smp_url"):
        print(f"SMP host URI    : {r['smp_url']}")
    if r.get("smp_hostname"):
        print(f"SMP hostname    : {r['smp_hostname']}")
    if r.get("error"):
        print(f"ERROR           : {r['error']}")
        return
    if "access_points" not in r:
        # ap-only, ou participant non enregistré : on s'arrête avant le smpquery
        return
    if not r.get("access_points"):
        print(f"\nAucun Access Point trouvé.")
        return
    print(f"\nAccess Points ({len(r['access_points'])}) :")
    for ap in r["access_points"]:
        print()
        print(f"  ┏━ {ap['ap_peppol_id']}")
        print(f"  ┃ Organisation     : {ap['organization']} ({ap['country']})")
        if ap['organizational_unit']:
            print(f"  ┃ OU               : {ap['organizational_unit']}")
        print(f"  ┃ Certificate CA   : {ap['issuer_ca']}")
        print(f"  ┃ Certificate valid: {ap['cert_valid_from']} → {ap['cert_valid_to']}")
        print(f"  ┃ Endpoint URLs    : {len(ap['endpoint_urls'])}")
        for url in ap["endpoint_urls"]:
            print(f"  ┃   • {url}")
        print(f"  ┃ Transport        : {', '.join(ap['transport_profiles'])}")
        print(f"  ┃ Doctypes supp.   : {len(ap['doctypes_supported'])}")
        if ap["technical_contact_urls"]:
            print(f"  ┃ Tech contact     : {', '.join(ap['technical_contact_urls'])}")
        print(f"  ┗━")


def main() -> int:
    ap = argparse.ArgumentParser(
        description="Résout un Participant ID Peppol vers les infos de l'AP/PA "
                    "via l'API REST publique de Helger (SML+SMP côté serveur).",
    )
    ap.add_argument("participant", help="Format '0225:000122308' ou 'iso6523-actorid-upis::0225:000122308'")
    ap.add_argument("--test", action="store_true", help="Use SMK (test SML) au lieu de la production")
    ap.add_argument("--full", action="store_true", help="Sortie JSON complète (incluant tous les endpoints bruts)")
    ap.add_argument("--summary", action="store_true", help="Sortie courte lisible (défaut)")
    ap.add_argument(
        "--proxy",
        default=None,
        help="Proxy HTTP(S), ex. http://user:pass@proxy.corp:8080. "
             "À défaut, HTTP_PROXY/HTTPS_PROXY/NO_PROXY sont utilisés.",
    )
    ap.add_argument(
        "--ca-bundle",
        default=None,
        help="Chemin du CA bundle d'entreprise (interception TLS). "
             "À défaut: SSL_CERT_FILE / REQUESTS_CA_BUNDLE / CURL_CA_BUNDLE.",
    )
    ap.add_argument(
        "--insecure",
        action="store_true",
        help="Désactive la vérification TLS (à éviter; utile pour diagnostiquer).",
    )
    ap.add_argument(
        "--debug",
        action="store_true",
        help="Trace toutes les requêtes HTTP (extrait body) sur stderr.",
    )
    ap.add_argument(
        "--ap-only",
        action="store_true",
        help="Ne fait que ppidexistence et s'arrête : sortie {participant, "
             "existence, smp_url, smp_hostname}. Une seule requête.",
    )
    args = ap.parse_args()

    global _DEBUG
    _DEBUG = args.debug

    try:
        scheme, value = parse_pid(args.participant)
    except ValueError as e:
        print(f"ERROR: {e}", file=sys.stderr)
        return 2

    configure_network(
        proxy=args.proxy,
        ca_bundle=args.ca_bundle,
        insecure=args.insecure,
    )

    sml_id = SML_TEST if args.test else SML_PROD
    result = resolve(scheme, value, sml_id, ap_only=args.ap_only)

    if args.full:
        print(json.dumps(result, indent=2, ensure_ascii=False, default=str))
    else:
        print_summary(result)
    return 0


if __name__ == "__main__":
    sys.exit(main())
