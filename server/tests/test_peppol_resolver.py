"""peppol_resolver : retry DNS sur erreur transitoire, pas sur réponse définitive.

Sans retry, un timeout ou un SERVFAIL isolé (RRL de l'autoritaire SML sous
rafale) devient un DNS_ERROR définitif — que l'API transformait en faux
« absent de Peppol » (constaté en prod le 2026-07-13).
"""
import types
import unittest

import dns.exception
import dns.resolver

import peppol_resolver


def _naptr(url: str):
    """Rdata NAPTR factice, suffisant pour resolve_smp_url."""
    return types.SimpleNamespace(
        order=100, preference=10, flags=b"U",
        service=b"Meta:SMP", regexp=f"!.*!{url}!".encode(),
    )


class _FakeResolver:
    """resolve() rejoue un scénario : liste d'exceptions à lever, puis succès."""

    def __init__(self, failures, answers):
        self.failures = list(failures)
        self.answers = answers
        self.calls = 0

    def resolve(self, name, rdtype, lifetime=None):
        self.calls += 1
        if self.failures:
            raise self.failures.pop(0)
        return self.answers


class ResolveSmpUrlRetryTests(unittest.TestCase):
    def setUp(self):
        self._orig_resolver = peppol_resolver._DNS_RESOLVER
        self._orig_fallback = peppol_resolver._DNS_FALLBACK
        self._orig_base = peppol_resolver.DNS_RETRY_BASE
        peppol_resolver.DNS_RETRY_BASE = 0  # pas d'attente réelle en test
        peppol_resolver._DNS_FALLBACK = None

    def tearDown(self):
        peppol_resolver._DNS_RESOLVER = self._orig_resolver
        peppol_resolver._DNS_FALLBACK = self._orig_fallback
        peppol_resolver.DNS_RETRY_BASE = self._orig_base

    def _run(self, fake):
        peppol_resolver._DNS_RESOLVER = fake
        return peppol_resolver.resolve_smp_url("iso6523-actorid-upis", "0225:1",
                                               peppol_resolver.SML_PROD)

    def test_transitoire_puis_succes(self):
        # Deux timeouts puis une réponse : le lookup doit aboutir.
        fake = _FakeResolver(
            [dns.exception.Timeout(), dns.exception.Timeout()],
            [_naptr("https://smp.example.org")],
        )
        out = self._run(fake)
        self.assertEqual(out["status"], "OK")
        self.assertEqual(out["smp_url"], "https://smp.example.org")
        self.assertEqual(fake.calls, 3)

    def test_noanswer_transitoire_puis_succes(self):
        # Constaté en prod le 2026-07-13 (après le 1er correctif) : sous rafale,
        # 32/40 NoAnswer en batch alors que les MÊMES adressages répondent OK en
        # unitaire une seconde après. NoAnswer est donc un artefact transitoire
        # de rafale, à re-tenter — seul NXDOMAIN est une négation fiable.
        fake = _FakeResolver(
            [dns.resolver.NoAnswer(), dns.resolver.NoAnswer()],
            [_naptr("https://smp.example.org")],
        )
        out = self._run(fake)
        self.assertEqual(out["status"], "OK")
        self.assertEqual(fake.calls, 3)

    def test_noanswer_persistant_garde_son_statut(self):
        # Après épuisement des tentatives, le statut reste « NoAnswer »
        # (distinct d'un DNS_ERROR) : l'API le classe en échec de lookup.
        fake = _FakeResolver([dns.resolver.NoAnswer()] * 10, [])
        out = self._run(fake)
        self.assertEqual(out["status"], "NoAnswer")
        self.assertEqual(fake.calls, 1 + peppol_resolver.DNS_MAX_RETRIES)

    def test_nxdomain_definitif_sans_retry(self):
        # NXDOMAIN est la réponse authentique « non enregistré » : la re-tenter
        # gaspillerait le quota DNS pour un résultat identique.
        fake = _FakeResolver([dns.resolver.NXDOMAIN()], [])
        out = self._run(fake)
        self.assertEqual(out["status"], "NXDOMAIN")
        self.assertEqual(fake.calls, 1)

    def test_fallback_sauve_un_noanswer_persistant(self):
        # Incident 2026-07-13 (2e étage) : le cache négatif du résolveur
        # principal reste empoisonné 15 min (TTL SOA de la zone CEF) — les
        # retries en secondes n'y peuvent rien. Un résolveur de SECOURS
        # (autre cache) doit être consulté en ultime recours.
        primary = _FakeResolver([dns.resolver.NoAnswer()] * 10, [])
        fallback = _FakeResolver([], [_naptr("https://smp.example.org")])
        peppol_resolver._DNS_FALLBACK = fallback
        out = self._run(primary)
        self.assertEqual(out["status"], "OK")
        self.assertEqual(out["smp_url"], "https://smp.example.org")
        self.assertEqual(primary.calls, 1 + peppol_resolver.DNS_MAX_RETRIES)
        self.assertEqual(fallback.calls, 1)

    def test_fallback_nxdomain_fait_foi(self):
        # Le secours répond NXDOMAIN (réponse authentique) : verdict « non
        # enregistré », pas un échec de lookup.
        primary = _FakeResolver([dns.resolver.NoAnswer()] * 10, [])
        peppol_resolver._DNS_FALLBACK = _FakeResolver([dns.resolver.NXDOMAIN()], [])
        out = self._run(primary)
        self.assertEqual(out["status"], "NXDOMAIN")

    def test_fallback_en_echec_conserve_le_statut_principal(self):
        primary = _FakeResolver([dns.resolver.NoAnswer()] * 10, [])
        peppol_resolver._DNS_FALLBACK = _FakeResolver([dns.exception.Timeout()] * 10, [])
        out = self._run(primary)
        self.assertEqual(out["status"], "NoAnswer")

    def test_nxdomain_principal_ne_consulte_pas_le_fallback(self):
        # NXDOMAIN du principal est définitif : consulter le secours doublerait
        # le trafic DNS pour les ~70 % d'adressages légitimement absents.
        primary = _FakeResolver([dns.resolver.NXDOMAIN()], [])
        fallback = _FakeResolver([], [_naptr("https://smp.example.org")])
        peppol_resolver._DNS_FALLBACK = fallback
        out = self._run(primary)
        self.assertEqual(out["status"], "NXDOMAIN")
        self.assertEqual(fallback.calls, 0)

    def test_configure_network_construit_le_fallback(self):
        peppol_resolver.configure_network(dns_fallback="8.8.8.8")
        self.assertIsNotNone(peppol_resolver._DNS_FALLBACK)
        self.assertEqual(peppol_resolver._DNS_FALLBACK.nameservers, ["8.8.8.8"])
        peppol_resolver.configure_network()  # sans fallback : désactivé
        self.assertIsNone(peppol_resolver._DNS_FALLBACK)

    def test_echec_persistant_reste_une_erreur(self):
        # Toutes les tentatives échouent : DNS_ERROR (l'API le remontera en
        # erreur re-tentable, jamais en exists:false).
        fake = _FakeResolver([dns.exception.Timeout()] * 10, [])
        out = self._run(fake)
        self.assertTrue(out["status"].startswith("DNS_ERROR"))
        self.assertEqual(fake.calls, 1 + peppol_resolver.DNS_MAX_RETRIES)


if __name__ == "__main__":
    unittest.main()
