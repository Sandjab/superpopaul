#!/usr/bin/env python3
"""
Peppol Participant Resolver — du scheme:value à la fiche complète des Access Points.

Pipeline :
  1. SHA-256 + base32 sur lowercase(value) → hostname dans le SML
  2. DNS NAPTR sur participant.sml.prod.tech.peppol.org → URL du SMP
  3. GET <smp_url>/<urlencode(pid)> → ServiceGroup (liste des doctypes supportés)
  4. Pour chaque ServiceMetadataReference, GET → ServiceMetadata
  5. Extrait pour chaque Endpoint : URL AS4 + certificat X.509
  6. Parse le cert → identification de l'AP (= PA française dans le contexte CTC)

Usage :
    python peppol_resolver.py 0225:000122308
    python peppol_resolver.py iso6523-actorid-upis::0225:000122308
    python peppol_resolver.py 0225:000122308 --full      # JSON brut complet
    python peppol_resolver.py 0225:000122308 --test      # SMK test SML

Dépendances : dnspython, cryptography (pip install dnspython cryptography)
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import random
import re
import ssl
import sys
import threading
import time
import urllib.error
import urllib.request
from typing import Any
import urllib.parse
from urllib.parse import quote
from xml.etree import ElementTree as ET

import dns.message
import dns.rcode
import dns.rdatatype
import dns.resolver
from cryptography import x509
from cryptography.hazmat.backends import default_backend
from cryptography.hazmat.primitives import hashes as crypto_hashes

# OpenPeppol SML insourcing (migration nov 2025 → août 2026)
SML_PROD = "participant.sml.prod.tech.peppol.org"
SML_TEST = "participant.sml.test.tech.peppol.org"
DEFAULT_SCHEME = "iso6523-actorid-upis"

DEFAULT_DOH_ENDPOINT = "https://cloudflare-dns.com/dns-query"

HTTP_TIMEOUT = 20
DNS_TIMEOUT = 10
UA = "peppol-resolver/1.0 (+research)"

# Retry avec backoff sur les statuts transitoires (l'annuaire Peppol renvoie
# 429 quand on le sollicite trop vite en parallèle). Retry-After respecté.
HTTP_RETRY_STATUSES = frozenset({429, 503})
HTTP_MAX_RETRIES = 4
HTTP_RETRY_BASE = 1.0          # secondes (×2 à chaque tentative + jitter)

# DNS NAPTR : mêmes précautions que le HTTP. L'autoritaire SML fait du
# Response Rate Limiting : sous rafale il droppe (timeout) ou SERVFAIL —
# transitoire, donc re-tentable. NXDOMAIN/NoAnswer sont des réponses
# définitives, jamais re-tentées. Le sémaphore borne les lookups simultanés
# (indépendamment de la concurrence HTTP de l'appelant) : c'est la rafale
# DNS qui déclenche le RRL, constaté en prod le 2026-07-13.
DNS_MAX_RETRIES = 2            # tentatives supplémentaires sur erreur transitoire
DNS_RETRY_BASE = 0.5           # secondes (×2 à chaque tentative + jitter)
DNS_MAX_CONCURRENCY = 16
_DNS_SEM = threading.BoundedSemaphore(DNS_MAX_CONCURRENCY)

_DEBUG = False


def _dbg(msg: str) -> None:
    if _DEBUG:
        print(f"[debug] {msg}", file=sys.stderr)

# Initialized by configure_network(); fall back to defaults if never called.
_HTTP_OPENER: urllib.request.OpenerDirector | None = None
_DNS_RESOLVER: Any = None  # dns.resolver.Resolver or _DoHResolver
# Résolveur de SECOURS (autre cache) consulté en ultime recours quand le
# principal échoue après retries : son cache négatif à lui n'est presque
# jamais empoisonné pour les mêmes noms au même moment (incident 2026-07-13 :
# NODATA mis en cache 15 min — TTL SOA CEF — par le POP du résolveur primaire).
_DNS_FALLBACK: Any = None


def _resolve_ca_bundle(explicit: str | None) -> str | None:
    """Pick a CA bundle path from CLI arg or common env vars."""
    if explicit:
        return explicit
    for var in ("SSL_CERT_FILE", "REQUESTS_CA_BUNDLE", "CURL_CA_BUNDLE"):
        v = os.environ.get(var)
        if v:
            return v
    return None


class _DoHResolver:
    """Minimal DNS-over-HTTPS resolver that reuses the global HTTP opener.

    Built so corporate environments where outbound UDP/53 is blocked but
    HTTPS is proxied can still perform NAPTR lookups: DoH traffic flows
    through the same proxy and trusts the same CA bundle as everything else.
    """

    def __init__(self, endpoint: str):
        self.endpoint = endpoint

    def resolve(self, name, rdtype, lifetime=None):
        rtype = dns.rdatatype.from_text(rdtype) if isinstance(rdtype, str) else rdtype
        q = dns.message.make_query(name, rtype)
        wire = q.to_wire()
        b64 = base64.urlsafe_b64encode(wire).rstrip(b"=").decode("ascii")
        url = f"{self.endpoint}?dns={b64}"
        req = urllib.request.Request(
            url,
            headers={"Accept": "application/dns-message", "User-Agent": UA},
        )
        _dbg(f"DoH GET {self.endpoint} (NAPTR {name})")
        try:
            with _opener().open(req, timeout=lifetime or HTTP_TIMEOUT) as resp:
                body = resp.read()
                _dbg(f"  < {resp.status} {resp.reason} ({len(body)} bytes)")
        except urllib.error.HTTPError as e:
            err = e.read()[:200].decode("utf-8", errors="replace")
            _dbg(f"  < HTTP {e.code} {e.reason}: {err!r}")
            raise
        except urllib.error.URLError as e:
            _dbg(f"  < URLError: {e.reason!r}")
            raise
        msg = dns.message.from_wire(body)
        if msg.rcode() == dns.rcode.NXDOMAIN:
            raise dns.resolver.NXDOMAIN()
        for rrset in msg.answer:
            if rrset.rdtype == rtype:
                return rrset
        raise dns.resolver.NoAnswer(response=msg)


def configure_network(
    proxy: str | None = None,
    ca_bundle: str | None = None,
    insecure: bool = False,
    dns_server: str | None = None,
    doh_endpoint: str | None = None,
    dns_fallback: str | None = None,
) -> None:
    """Build the global HTTP opener and DNS resolver used by the resolver.

    Proxy: explicit URL overrides env (HTTP_PROXY/HTTPS_PROXY/NO_PROXY). If both
    are unset, urllib uses a direct connection.

    CA bundle: explicit path overrides SSL_CERT_FILE/REQUESTS_CA_BUNDLE/
    CURL_CA_BUNDLE. Required when the corporate proxy does TLS interception.

    DNS: explicit nameserver bypasses /etc/resolv.conf (useful when the
    corporate resolver blocks public NAPTR lookups).
    """
    global _HTTP_OPENER, _DNS_RESOLVER, _DNS_FALLBACK

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

    if doh_endpoint:
        _DNS_RESOLVER = _DoHResolver(doh_endpoint)
    elif dns_server:
        r = dns.resolver.Resolver(configure=False)
        r.nameservers = [dns_server]
        r.lifetime = DNS_TIMEOUT
        _DNS_RESOLVER = r
    else:
        _DNS_RESOLVER = dns.resolver.Resolver()
        _DNS_RESOLVER.lifetime = DNS_TIMEOUT

    if dns_fallback:
        fb = dns.resolver.Resolver(configure=False)
        fb.nameservers = [dns_fallback]
        fb.lifetime = DNS_TIMEOUT
        _DNS_FALLBACK = fb
    else:
        _DNS_FALLBACK = None


def _opener() -> urllib.request.OpenerDirector:
    if _HTTP_OPENER is None:
        configure_network()
    assert _HTTP_OPENER is not None
    return _HTTP_OPENER


def _resolver() -> Any:
    if _DNS_RESOLVER is None:
        configure_network()
    assert _DNS_RESOLVER is not None
    return _DNS_RESOLVER


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


def sml_hostname(scheme: str, value: str, sml_zone: str) -> str:
    """Build NAPTR lookup hostname per OpenPeppol SML spec (post-Nov 2025).

    Format : strip_trailing(base32(sha256(lowercase(VALUE))), '=') + '.' + SCHEME + '.' + ZONE
    """
    digest = hashlib.sha256(value.lower().encode("utf-8")).digest()
    b32 = base64.b32encode(digest).decode("ascii").rstrip("=").lower()
    return f"{b32}.{scheme}.{sml_zone}"


def resolve_smp_url(scheme: str, value: str, sml_zone: str) -> dict[str, Any]:
    """DNS NAPTR lookup. Returns dict with smp_url + raw NAPTR records."""
    host = sml_hostname(scheme, value, sml_zone)
    out: dict[str, Any] = {"hostname": host, "status": None, "smp_url": None, "naptr_records": []}
    answers = None
    for attempt in range(DNS_MAX_RETRIES + 1):
        try:
            # Sémaphore relâché pendant le sleep de backoff : un slot n'est
            # tenu que le temps du lookup lui-même.
            with _DNS_SEM:
                answers = _resolver().resolve(host, "NAPTR", lifetime=DNS_TIMEOUT)
            break
        except dns.resolver.NXDOMAIN:
            # Seule réponse négative fiable : « non enregistré », définitif.
            out["status"] = "NXDOMAIN"
            return out
        except Exception as e:
            # NoAnswer compris : sous rafale, l'autoritaire SML rend des
            # réponses vides transitoires (constaté en prod le 2026-07-13,
            # les mêmes adressages répondent OK en unitaire juste après).
            if attempt < DNS_MAX_RETRIES:
                delay = DNS_RETRY_BASE * (2 ** attempt) + random.uniform(0, 0.25)
                _dbg(f"NAPTR {host}: {type(e).__name__}, retry {attempt + 1}/{DNS_MAX_RETRIES} dans {delay:.1f}s")
                time.sleep(delay)
                continue
            out["status"] = ("NoAnswer" if isinstance(e, dns.resolver.NoAnswer)
                             else f"DNS_ERROR:{type(e).__name__}: {e}")

    if answers is None and _DNS_FALLBACK is not None:
        # Échec après retries : le cache négatif du résolveur principal est
        # peut-être empoisonné (NODATA gardé 15 min — TTL SOA CEF, incident
        # 2026-07-13). Ultime tentative sur un AUTRE cache ; son NXDOMAIN
        # fait foi, tout autre échec conserve le statut principal.
        try:
            with _DNS_SEM:
                answers = _DNS_FALLBACK.resolve(host, "NAPTR", lifetime=DNS_TIMEOUT)
            _dbg(f"NAPTR {host}: sauvé par le résolveur de secours")
        except dns.resolver.NXDOMAIN:
            out["status"] = "NXDOMAIN"
            return out
        except Exception as e:
            _dbg(f"NAPTR {host}: secours en échec aussi ({type(e).__name__})")

    if answers is None:
        return out

    out["status"] = "OK"
    for rdata in answers:
        service = rdata.service.decode()
        regexp = rdata.regexp.decode()
        # Peppol NAPTR regexp : '!.*!<smp_url>!'
        m = re.match(r"!\.\*!([^!]+)!", regexp)
        url = m.group(1) if m else None
        out["naptr_records"].append({
            "order": rdata.order,
            "preference": rdata.preference,
            "flags": rdata.flags.decode(),
            "service": service,
            "regexp": regexp,
            "extracted_url": url,
        })
        if service.startswith("Meta:SMP") and url and not out["smp_url"]:
            out["smp_url"] = url
    return out


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


def http_get(url: str, accept: str = "application/xml", ua: str = UA) -> str:
    req = urllib.request.Request(url, headers={"Accept": accept, "User-Agent": ua})
    _dbg(f"GET {url}")
    _dbg(f"  > Accept: {accept}")
    _dbg(f"  > User-Agent: {ua}")
    for attempt in range(HTTP_MAX_RETRIES + 1):
        try:
            with _opener().open(req, timeout=HTTP_TIMEOUT) as resp:
                body = resp.read().decode("utf-8", errors="replace")
                _dbg(f"  < {resp.status} {resp.reason}")
                for k, v in resp.headers.items():
                    _dbg(f"  < {k}: {v}")
                _dbg(f"  < body[:300] = {body[:300]!r}")
                return body
        except urllib.error.HTTPError as e:
            _dbg(f"  < HTTP {e.code} {e.reason}")
            for k, v in e.headers.items():
                _dbg(f"  < {k}: {v}")
            try:
                err_body = e.read().decode("utf-8", errors="replace")
                _dbg(f"  < body[:300] = {err_body[:300]!r}")
            except Exception:
                pass
            if e.code in HTTP_RETRY_STATUSES and attempt < HTTP_MAX_RETRIES:
                delay = _retry_after_seconds(e, attempt)
                _dbg(f"  ↻ retry {attempt + 1}/{HTTP_MAX_RETRIES} dans {delay:.1f}s")
                time.sleep(delay)
                continue
            raise


# (User-Agent, Accept, urlencode-colons?) variants used by --debug to diagnose 4xx
_SG_PROBES = [
    (UA, "application/xml", True),
    (UA, "application/xml", False),
    (UA, "text/xml", True),
    (UA, "*/*", True),
    ("curl/8.4.0", "*/*", True),
    (
        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) "
        "Chrome/124.0 Safari/537.36",
        "application/xml,text/xml;q=0.9,*/*;q=0.5",
        True,
    ),
]


def probe_service_group(smp_url: str, pid_full: str) -> list[dict[str, Any]]:
    """Try a matrix of (UA, Accept, encoding) variants and report each result."""
    base = smp_url.rstrip("/")
    encoded = quote(pid_full, safe="")
    raw = pid_full  # colons left intact
    results: list[dict[str, Any]] = []
    for ua, accept, do_encode in _SG_PROBES:
        url = f"{base}/{encoded if do_encode else raw}"
        attempt: dict[str, Any] = {
            "url": url, "user_agent": ua, "accept": accept,
            "url_encoded_pid": do_encode,
        }
        try:
            body = http_get(url, accept=accept, ua=ua)
            attempt["status"] = "OK"
            attempt["body_preview"] = body[:200]
        except urllib.error.HTTPError as e:
            attempt["status"] = f"HTTP {e.code}"
        except Exception as e:
            attempt["status"] = f"{type(e).__name__}: {e}"
        results.append(attempt)
    return results


def _local(tag: str) -> str:
    """Strip XML namespace from tag."""
    return tag.split("}", 1)[-1] if "}" in tag else tag


def fetch_service_group(smp_url: str, pid_full: str) -> list[str]:
    """Returns list of ServiceMetadataReference hrefs from the ServiceGroup."""
    url = f"{smp_url.rstrip('/')}/{quote(pid_full, safe='')}"
    xml = http_get(url)
    root = ET.fromstring(xml)
    return [el.get("href", "") for el in root.iter()
            if _local(el.tag) == "ServiceMetadataReference" and el.get("href")]


def parse_cert(b64_str: str) -> dict[str, Any]:
    """Parse DER-encoded X.509 cert from base64 (with or without whitespace).

    Tolerates PEM armor (`-----BEGIN CERTIFICATE-----` … `-----END …-----`):
    some SMPs wrap the cert that way inside <Certificate>, which would
    otherwise break base64 decoding once whitespace is stripped.
    """
    body = b64_str
    if "-----BEGIN" in body:
        body = "\n".join(
            ln for ln in body.splitlines() if not ln.strip().startswith("-----")
        )
    cleaned = "".join(body.split())
    der = base64.b64decode(cleaned)
    cert = x509.load_der_x509_certificate(der, default_backend())

    def attr(name, oid_name):
        return next((a.value for a in name if a.oid._name == oid_name), None)

    return {
        "subject_common_name": attr(cert.subject, "commonName"),
        "subject_organization": attr(cert.subject, "organizationName"),
        "subject_organizational_unit": attr(cert.subject, "organizationalUnitName"),
        "subject_country": attr(cert.subject, "countryName"),
        "issuer_common_name": attr(cert.issuer, "commonName"),
        "issuer_organization": attr(cert.issuer, "organizationName"),
        "serial_number": str(cert.serial_number),
        "not_valid_before": cert.not_valid_before_utc.isoformat(),
        "not_valid_after": cert.not_valid_after_utc.isoformat(),
        "fingerprint_sha256": cert.fingerprint(crypto_hashes.SHA256()).hex(),
    }


def fetch_service_metadata(href: str) -> list[dict[str, Any]]:
    """Returns list of endpoints (one per Process×Endpoint) with parsed cert."""
    xml = http_get(href)
    root = ET.fromstring(xml)

    endpoints: list[dict[str, Any]] = []
    # Trouver ServiceInformation (wrapped dans SignedServiceMetadata > ServiceMetadata)
    for si in root.iter():
        if _local(si.tag) != "ServiceInformation":
            continue
        doc_id_elt = next((e for e in si.iter() if _local(e.tag) == "DocumentIdentifier"), None)
        doc_id = doc_id_elt.text if doc_id_elt is not None else None
        doc_scheme = doc_id_elt.get("scheme") if doc_id_elt is not None else None

        for proc in si.iter():
            if _local(proc.tag) != "Process":
                continue
            pid_elt = next((e for e in proc.iter() if _local(e.tag) == "ProcessIdentifier"), None)
            process_id = pid_elt.text if pid_elt is not None else None

            for ep in proc.iter():
                if _local(ep.tag) != "Endpoint":
                    continue
                fields = {
                    "document_identifier": doc_id,
                    "document_scheme": doc_scheme,
                    "process_identifier": process_id,
                    "transport_profile": ep.get("transportProfile"),
                    "endpoint_url": None,
                    "require_business_signature": None,
                    "service_activation_date": None,
                    "service_expiration_date": None,
                    "service_description": None,
                    "technical_contact_url": None,
                    "certificate": None,
                }
                cert_b64 = None
                for child in ep.iter():
                    tag = _local(child.tag)
                    text = (child.text or "").strip()
                    if tag == "Address":
                        fields["endpoint_url"] = text
                    elif tag == "Certificate":
                        cert_b64 = text
                    elif tag == "RequireBusinessLevelSignature":
                        fields["require_business_signature"] = text
                    elif tag == "ServiceActivationDate":
                        fields["service_activation_date"] = text
                    elif tag == "ServiceExpirationDate":
                        fields["service_expiration_date"] = text
                    elif tag == "ServiceDescription":
                        fields["service_description"] = text
                    elif tag == "TechnicalContactUrl":
                        fields["technical_contact_url"] = text
                if cert_b64:
                    try:
                        fields["certificate"] = parse_cert(cert_b64)
                    except Exception as e:
                        fields["certificate"] = {"error": f"{type(e).__name__}: {e}"}
                endpoints.append(fields)
    return endpoints


def resolve(
    scheme: str,
    value: str,
    sml_zone: str = SML_PROD,
    ap_only: bool = False,
) -> dict[str, Any]:
    pid_full = f"{scheme}::{value}"
    result: dict[str, Any] = {
        "participant_id": pid_full,
        "scheme": scheme,
        "value": value,
        "sml_zone": sml_zone,
    }

    sml = resolve_smp_url(scheme, value, sml_zone)
    result["sml"] = sml
    if sml["status"] != "OK" or not sml["smp_url"]:
        return result

    smp_url = sml["smp_url"]
    result["smp_url"] = smp_url
    result["smp_hostname"] = urllib.parse.urlparse(smp_url).hostname

    if ap_only:
        return result

    try:
        refs = fetch_service_group(smp_url, pid_full)
    except urllib.error.HTTPError as e:
        result["error"] = f"ServiceGroup HTTP {e.code} on {smp_url}"
        if _DEBUG:
            result["service_group_probes"] = probe_service_group(smp_url, pid_full)
        return result
    except Exception as e:
        result["error"] = f"ServiceGroup fetch: {type(e).__name__}: {e}"
        if _DEBUG:
            result["service_group_probes"] = probe_service_group(smp_url, pid_full)
        return result

    result["service_metadata_refs_count"] = len(refs)

    all_endpoints: list[dict[str, Any]] = []
    for href in refs:
        try:
            all_endpoints.extend(fetch_service_metadata(href))
        except Exception as e:
            all_endpoints.append({"_fetch_error": f"{href}: {type(e).__name__}"})

    result["endpoints"] = all_endpoints

    # Synthèse : agrège par Access Point (clé = subject_common_name du cert)
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
    sml = r.get("sml", {})
    print(f"SML hostname    : {sml.get('hostname', '?')}")
    print(f"SML lookup      : {sml.get('status', '?')}")
    if r.get("smp_url"):
        print(f"SMP URL         : {r['smp_url']}")
    if r.get("smp_hostname"):
        print(f"SMP hostname    : {r['smp_hostname']}")
    if r.get("error"):
        print(f"ERROR           : {r['error']}")
        return
    if "access_points" not in r:
        # ap-only mode: stopped before fetching ServiceGroup
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
        description="Résout un Participant ID Peppol vers les infos de l'AP/PA via SML+SMP.",
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
        "--dns-server",
        default=None,
        help="Serveur DNS à utiliser pour les lookups NAPTR "
             "(ex. 8.8.8.8). À défaut: /etc/resolv.conf.",
    )
    ap.add_argument(
        "--doh",
        nargs="?",
        const=DEFAULT_DOH_ENDPOINT,
        default=None,
        metavar="ENDPOINT",
        help=f"Active DNS-over-HTTPS (passe par le proxy et le CA bundle). "
             f"Sans valeur: {DEFAULT_DOH_ENDPOINT}. Override --dns-server. "
             f"Indispensable derrière un proxy d'entreprise qui bloque "
             f"le DNS sortant ou ne propage pas les NAPTR externes.",
    )
    ap.add_argument(
        "--debug",
        action="store_true",
        help="Trace toutes les requêtes HTTP (headers + extrait body) sur stderr. "
             "En cas d'échec du ServiceGroup, sonde plusieurs variantes "
             "(UA, Accept, encodage du PID) pour cerner la cause.",
    )
    ap.add_argument(
        "--ap-only",
        action="store_true",
        help="Ne fait que SML→SMP et s'arrête : sortie {participant, sml, smp_url, "
             "smp_hostname}. Évite les 403 sur les SMP qui n'exposent pas le "
             "ServiceGroup publiquement. Utile pour batcher la réconciliation.",
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
        dns_server=args.dns_server,
        doh_endpoint=args.doh,
    )

    zone = SML_TEST if args.test else SML_PROD
    result = resolve(scheme, value, zone, ap_only=args.ap_only)

    if args.full:
        print(json.dumps(result, indent=2, ensure_ascii=False, default=str))
    else:
        print_summary(result)
    return 0


if __name__ == "__main__":
    sys.exit(main())
