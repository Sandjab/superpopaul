#!/usr/bin/env python3
"""Ventilation du coût CPU d'une résolution Peppol (chantier « 27 ms »).

cProfile armé sur time.process_time : seules les millisecondes CPU comptent,
les attentes réseau (dominantes en mur) pèsent ~0. Rejoue un mix de PIDs réels
et sérialise chaque résultat en JSON comme le ferait l'API, puis agrège
tottime par catégorie de module. L'overhead cProfile gonfle les fonctions
bytecode (~+40 % constaté) : le total « contrôle » affiché en fin de run est
mesuré sans profileur, c'est lui qui fait foi.

Usage :
    python3 tools/profil_resolution.py                 # mix par défaut
    python3 tools/profil_resolution.py --dns 8.8.8.8 --passes 3 0208:0870480184 …

Dépendances : celles de peppol_resolver (requirements.txt).
"""
import argparse
import cProfile
import json
import os
import pstats
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import peppol_resolver as pr  # noqa: E402

# Attention : des PIDs se font déradier du SML avec le temps — vérifier le
# compteur « SML OK » et rafraîchir cette liste si besoin.
DEFAULT_PIDS = [
    "0225:000122308", "0225:931153688", "0225:12345678900011",
    "0208:0870480184", "9925:be0400476017",
    "0007:5567321707", "0192:991825827",
]

CATEGORIES = [
    ("XML (ElementTree)", ("xml/etree", "ElementTree", "expat")),
    ("TLS (_ssl)", ("ssl.py", "_ssl.", "SSLSocket", "SSLContext", "do_handshake")),
    ("X.509 (cryptography)", ("cryptography", "parse_cert", "x509")),
    ("DNS (dnspython)", ("/dns/", "dns.message")),
    ("getaddrinfo/connect", ("getaddrinfo", "_socket.socket",)),
    ("HTTP (urllib/http.client)", ("urllib/", "http/client", "email/")),
    ("JSON", ("json/", "_json")),
    ("socket/select", ("socket.py", "selectors", "select.")),
]


def categorize(key):
    path, _line, name = key
    hay = f"{path} {name}"
    for label, needles in CATEGORIES:
        if any(n in hay for n in needles):
            return label
    return "autre (bytecode resolver & stdlib)"


def run(pids, passes, profile):
    prof = cProfile.Profile(time.process_time) if profile else None
    ok = err = 0
    wall0, cpu0 = time.perf_counter(), time.process_time()
    for _ in range(passes):
        for scheme, value in pids:
            if prof:
                prof.enable()
            try:
                r = pr.resolve(scheme, value)
                json.dumps(r)  # sérialisation comme dans l'API
                ok += 1 if r.get("sml", {}).get("status") == "OK" else 0
            except Exception as e:  # on profile, on ne s'arrête pas
                err += 1
                print(f"ERREUR {scheme}:{value} : {e}", file=sys.stderr)
            finally:
                if prof:
                    prof.disable()
    wall = time.perf_counter() - wall0
    cpu = time.process_time() - cpu0
    return prof, ok, err, wall, cpu


def main():
    ap = argparse.ArgumentParser(
        description="Ventilation du coût CPU d'une résolution Peppol")
    ap.add_argument("pids", nargs="*", default=DEFAULT_PIDS,
                    help="PIDs bruts (défaut : mix du bench)")
    ap.add_argument("--dns", default="8.8.8.8",
                    help="serveur DNS (défaut 8.8.8.8 — les DNS de box "
                         "échouent souvent sur les NAPTR du SML)")
    ap.add_argument("--passes", type=int, default=3)
    args = ap.parse_args()

    pr.configure_network(dns_server=args.dns or None)
    # Le découpage passe par parse_pid, comme dans l'API : le hash SML porte
    # sur « 0225:000122308 » entier, avec le label de schéma complet.
    pids = [pr.parse_pid(p) for p in (args.pids or DEFAULT_PIDS)]
    n = args.passes * len(pids)

    prof, ok, err, wall, cpu = run(pids, args.passes, profile=True)
    print(f"\n{n} résolutions ({ok} SML OK, {err} erreurs) — "
          f"mur {wall:.1f}s, CPU {cpu*1000:.0f} ms → {cpu*1000/n:.1f} ms/résolution")

    st = pstats.Stats(prof)
    stats = st.stats  # type: ignore[attr-defined]  # attribut public non typé
    total = sum(tt for (_cc, _nc, tt, _ct, _cl) in stats.values()) or 1e-9
    buckets, calls = {}, {}
    for key, (cc, _nc, tt, _ct, _callers) in stats.items():
        buckets[categorize(key)] = buckets.get(categorize(key), 0.0) + tt
        if key[2] in ("http_get", "fetch_service_metadata", "fetch_service_group",
                      "parse_cert", "do_handshake"):
            calls[key[2]] = calls.get(key[2], 0) + cc
    print(f"\n{'Catégorie':<40}{'ms/résolution':>14}{'part':>8}")
    for label, tt in sorted(buckets.items(), key=lambda kv: -kv[1]):
        print(f"{label:<40}{tt*1000/n:>13.2f}{tt/total*100:>7.1f}%")
    print("\nAppels par résolution :")
    for fname, cc in sorted(calls.items()):
        print(f"  {fname:<24}{cc/n:>6.1f}")

    _, ok2, _, _, cpu2 = run(pids, args.passes, profile=False)
    print(f"\nContrôle sans profileur : {cpu2*1000/n:.1f} ms CPU/résolution "
          f"({cpu2*1000/max(ok2,1):.1f} ms par résolution SML OK) — fait foi.")


if __name__ == "__main__":
    main()
