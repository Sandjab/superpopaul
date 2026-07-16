"""peppol_api : auth par clé, rate-limit, mise en forme de la réponse simple.

Le résolveur réseau est remplacé par des sorties factices (aucun appel Peppol).
"""
import json
import threading
import unittest
import urllib.error
import urllib.request
from http.server import ThreadingHTTPServer

import peppol_api

# URN officiel PASR France §6.1.c (UBL EN16931 EXTENDED-CTC-FR), volontairement
# en littéral ici : le garde-fou ConstantTests vérifie que la constante inline
# de peppol_api n'en dérive pas.
FR_CTC_PRIMARY_INVOICE = (
    "urn:oasis:names:specification:ubl:schema:xsd:Invoice-2::Invoice##urn:cen.eu:en16931:2017#conformant#urn:peppol:france:billing:extended:1.0::2.1"
)


def _ap(code, name, country, doctypes):
    return {
        "ap_peppol_id": code,
        "organization": name,
        "country": country,
        "doctypes_supported": list(doctypes),
    }


# Sorties type de peppol_resolver.resolve(), suffisantes pour simple_view().
RESULT_OK_EXT = {
    "participant_id": "iso6523-actorid-upis::0225:ext",
    "scheme": "iso6523-actorid-upis", "value": "0225:ext",
    "sml": {"status": "OK"}, "smp_url": "https://smp.example/x",
    "access_points": [_ap("PFR000123", "Exemple SAS", "FR", [FR_CTC_PRIMARY_INVOICE, "other"])],
}
RESULT_OK_NOEXT = {
    "participant_id": "iso6523-actorid-upis::0225:noext",
    "scheme": "iso6523-actorid-upis", "value": "0225:noext",
    "sml": {"status": "OK"}, "smp_url": "https://smp.example/y",
    "access_points": [_ap("PFR000999", "Autre PA", "FR", ["something-else"])],
}
RESULT_ABSENT = {
    "participant_id": "iso6523-actorid-upis::0225:absent",
    "scheme": "iso6523-actorid-upis", "value": "0225:absent",
    "sml": {"status": "NXDOMAIN"}, "smp_url": None,
}
RESULT_DNS_FAIL = {
    "participant_id": "iso6523-actorid-upis::0225:dnsfail",
    "scheme": "iso6523-actorid-upis", "value": "0225:dnsfail",
    "sml": {"status": "DNS_ERROR:NoNameservers: SERVFAIL"}, "smp_url": None,
}


class SmlLookupErrorTests(unittest.TestCase):
    """Un échec de lookup SML (DNS_ERROR, NoAnswer) n'est PAS une absence :
    le déguiser en exists:false fabrique des faux négatifs silencieux (constaté
    en prod le 2026-07-13 sous rafale DNS). Seul NXDOMAIN — réponse authentique
    de l'autoritaire — signifie « non enregistré »."""

    def test_dns_error_est_un_echec_de_lookup(self):
        err = peppol_api.sml_lookup_error(RESULT_DNS_FAIL)
        self.assertIsNotNone(err)
        self.assertIn("DNS_ERROR", err)

    def test_noanswer_est_un_echec_de_lookup(self):
        r = dict(RESULT_DNS_FAIL, sml={"status": "NoAnswer"})
        self.assertIsNotNone(peppol_api.sml_lookup_error(r))

    def test_nxdomain_est_une_absence_pas_un_echec(self):
        self.assertIsNone(peppol_api.sml_lookup_error(RESULT_ABSENT))

    def test_ok_nest_pas_un_echec(self):
        self.assertIsNone(peppol_api.sml_lookup_error(RESULT_OK_EXT))


class SimpleViewTests(unittest.TestCase):
    def test_exists_with_extended(self):
        v = peppol_api.simple_view(RESULT_OK_EXT)
        self.assertTrue(v["exists"])
        self.assertTrue(v["supports_extended_ctc_fr"])
        self.assertEqual(v["pa"], {"code": "PFR000123", "name": "Exemple SAS", "country": "FR"})

    def test_exists_without_extended(self):
        v = peppol_api.simple_view(RESULT_OK_NOEXT)
        self.assertTrue(v["exists"])
        self.assertFalse(v["supports_extended_ctc_fr"])
        self.assertEqual(v["pa"]["code"], "PFR000999")

    def test_absent(self):
        v = peppol_api.simple_view(RESULT_ABSENT)
        self.assertFalse(v["exists"])
        self.assertFalse(v["supports_extended_ctc_fr"])
        self.assertIsNone(v["pa"])

    def test_registered_but_catalogue_unavailable(self):
        r = {"participant_id": "x", "scheme": "s", "value": "v",
             "sml": {"status": "OK"}, "smp_url": "https://smp/z",
             "error": "ServiceGroup HTTP 403"}
        v = peppol_api.simple_view(r)
        self.assertTrue(v["exists"])
        self.assertIsNone(v["supports_extended_ctc_fr"])
        self.assertIn("403", v["note"])


def _ep(doctype, activation=None, expiration=None):
    return {
        "document_identifier": doctype,
        "service_activation_date": activation,
        "service_expiration_date": expiration,
    }


class CtcServiceDatesTests(unittest.TestCase):
    """Étape de MESURE (2026-07-16) : les ServiceActivation/ExpirationDate
    déclarées sur le endpoint CTC sont tracées dans la note diagnostique,
    SANS toucher au verdict supports_extended_ctc_fr — on décidera d'un
    verdict temporel quand on saura si le phénomène existe en vrai."""

    def _ok_ext(self, endpoints):
        return dict(RESULT_OK_EXT, endpoints=endpoints)

    def test_activation_ctc_tracee_dans_note_verdict_inchange(self):
        v = peppol_api.simple_view(
            self._ok_ext([_ep(FR_CTC_PRIMARY_INVOICE, activation="2026-09-01")]))
        self.assertTrue(v["supports_extended_ctc_fr"])  # mesure, pas verdict
        self.assertIn("support CTC", v["note"])
        self.assertIn("activation 2026-09-01", v["note"])

    def test_expiration_ctc_tracee_dans_note(self):
        v = peppol_api.simple_view(
            self._ok_ext([_ep(FR_CTC_PRIMARY_INVOICE, expiration="2025-12-31")]))
        self.assertIn("expiration 2025-12-31", v["note"])

    def test_les_deux_dates_tracees(self):
        v = peppol_api.simple_view(self._ok_ext(
            [_ep(FR_CTC_PRIMARY_INVOICE, "2026-09-01", "2027-09-01")]))
        self.assertIn("activation 2026-09-01", v["note"])
        self.assertIn("expiration 2027-09-01", v["note"])

    def test_dates_des_autres_doctypes_ignorees(self):
        v = peppol_api.simple_view(
            self._ok_ext([_ep("other-doctype", "2020-01-01", "2021-01-01"),
                          _ep(FR_CTC_PRIMARY_INVOICE)]))
        self.assertIsNone(v.get("note"))

    def test_sans_dates_pas_de_note(self):
        # RESULT_OK_EXT n'a pas de clé endpoints : aucune note, pas d'erreur.
        v = peppol_api.simple_view(RESULT_OK_EXT)
        self.assertIsNone(v.get("note"))


class ConstantTests(unittest.TestCase):
    def test_ctc_constant_matches_spec_urn(self):
        """La constante inline de l'API doit rester l'URN officiel PASR
        §6.1.c (littéral en tête de ce fichier) : garde-fou anti-dérive."""
        self.assertEqual(
            peppol_api.FR_CTC_PRIMARY_INVOICE,
            FR_CTC_PRIMARY_INVOICE,
        )


class KeyTests(unittest.TestCase):
    def test_load_keys_inline_and_file(self):
        import tempfile, os
        with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False) as fh:
            fh.write("# comment\nclientA=aaa\nbbb\n\n")
            path = fh.name
        try:
            keys = peppol_api.load_keys("k1,k2", path, default_rate=60, default_burst=60)
        finally:
            os.unlink(path)
        self.assertEqual(keys["aaa"].label, "clientA")
        self.assertIn("bbb", keys)
        self.assertIn("k1", keys)
        self.assertIn("k2", keys)
        # Aucune de ces clés n'a de rate propre -> défaut.
        self.assertEqual(keys["aaa"].rate, 60)

    def test_load_keys_per_key_rate(self):
        import tempfile, os
        with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False) as fh:
            fh.write(
                "premium=ppp 600 100\n"    # rate + burst propres
                "std=sss\n"                # défaut
                "unlimited=uuu 0\n"        # illimité pour cette clé
                "burstdefault=bbb 120\n"   # rate propre, burst = rate
                "keepdefault=kkk - 5\n"    # rate défaut, burst 5
            )
            path = fh.name
        try:
            keys = peppol_api.load_keys(None, path, default_rate=60, default_burst=60)
        finally:
            os.unlink(path)
        self.assertEqual((keys["ppp"].rate, keys["ppp"].burst), (600, 100))
        self.assertEqual(keys["sss"].rate, 60)                 # défaut
        self.assertEqual(keys["uuu"].rate, 0)                  # illimité
        self.assertEqual((keys["bbb"].rate, keys["bbb"].burst), (120, 120))
        self.assertEqual((keys["kkk"].rate, keys["kkk"].burst), (60, 5))  # '-' -> défaut

    def test_load_keys_inline_rate(self):
        keys = peppol_api.load_keys("premium=ppp 300, std=sss", None,
                                    default_rate=60, default_burst=60)
        self.assertEqual(keys["ppp"].rate, 300)
        self.assertEqual(keys["sss"].rate, 60)

    def test_check_key(self):
        peppol_api.API_KEYS = {"secret": peppol_api.KeyConfig("clientX", 60, 60)}
        self.assertEqual(peppol_api.check_key("secret").label, "clientX")
        self.assertIsNone(peppol_api.check_key("nope"))
        self.assertIsNone(peppol_api.check_key(None))

    def test_check_key_rejects_oversized(self):
        # Une clé absurdement longue est rejetée avant toute comparaison (DoS CPU).
        peppol_api.API_KEYS = {"secret": peppol_api.KeyConfig("clientX", 60, 60)}
        self.assertIsNone(peppol_api.check_key("x" * 100_000))

    def test_extract_key(self):
        self.assertEqual(peppol_api.extract_key({"X-API-Key": "abc"}), "abc")
        self.assertEqual(peppol_api.extract_key({"Authorization": "Bearer xyz"}), "xyz")
        self.assertIsNone(peppol_api.extract_key({}))


class RateLimiterTests(unittest.TestCase):
    def test_burst_then_block(self):
        rl = peppol_api.RateLimiter()
        self.assertEqual(rl.allow("c", 60, 2)[0], True)
        self.assertEqual(rl.allow("c", 60, 2)[0], True)
        allowed, retry = rl.allow("c", 60, 2)
        self.assertFalse(allowed)
        self.assertGreater(retry, 0)

    def test_unlimited(self):
        rl = peppol_api.RateLimiter()
        for _ in range(100):
            self.assertTrue(rl.allow("c", 0, 1)[0])

    def test_per_key_independent(self):
        """Deux clients aux limites différentes ont des seaux indépendants."""
        rl = peppol_api.RateLimiter()
        self.assertTrue(rl.allow("small", 60, 1)[0])
        self.assertFalse(rl.allow("small", 60, 1)[0])         # seau 'small' vidé
        # 'big' a son propre seau (burst 3), non affecté par 'small'.
        self.assertTrue(rl.allow("big", 600, 3)[0])
        self.assertTrue(rl.allow("big", 600, 3)[0])
        self.assertTrue(rl.allow("big", 600, 3)[0])

    def test_cost_debits_that_many_tokens(self):
        """`cost` débite N jetons d'un coup : un appel à 9 vide presque le seau."""
        rl = peppol_api.RateLimiter()
        self.assertTrue(rl.allow("c", 60, 10, cost=9)[0])     # 10 -> 1
        self.assertTrue(rl.allow("c", 60, 10)[0])             # 1  -> 0
        self.assertFalse(rl.allow("c", 60, 10)[0])            # 0  -> refus

    def test_cost_above_burst_overdraws_then_blocks(self):
        """Découvert : un coût supérieur au pic passe s'il reste ≥ 1 jeton, met le
        solde en négatif, et le client attend d'avoir remboursé. Sans découvert un
        tel batch serait refusé à perpétuité (le seau ne tient que `burst`)."""
        rl = peppol_api.RateLimiter()
        allowed, _ = rl.allow("c", 60, 60, cost=500)          # seau plein (60) < 500
        self.assertTrue(allowed)
        allowed, retry = rl.allow("c", 60, 60)                # solde ≈ -440
        self.assertFalse(allowed)
        # 60 req/min = 1 jeton/s : il faut ~441 s pour revenir à 1 jeton.
        self.assertGreater(retry, 400)

    def test_cost_ignored_when_unlimited(self):
        rl = peppol_api.RateLimiter()
        for _ in range(10):
            self.assertTrue(rl.allow("c", 0, 1, cost=500)[0])

    def test_peek_ne_consomme_pas(self):
        """Consulter son solde ne doit pas le réduire — sinon /limits ferait
        payer l'observation."""
        rl = peppol_api.RateLimiter()
        self.assertEqual(rl.peek("c", 60, 5), 5.0)          # seau neuf = plein
        self.assertEqual(rl.peek("c", 60, 5), 5.0)
        rl.allow("c", 60, 5, cost=3)
        solde = rl.peek("c", 60, 5)
        assert solde is not None
        self.assertLess(solde, 2.1)                         # 5 - 3 = 2 (+ recharge)
        self.assertGreater(solde, 1.9)

    def test_peek_expose_la_dette(self):
        """Le découvert est visible : c'est l'intérêt d'exposer `current`."""
        rl = peppol_api.RateLimiter()
        rl.allow("c", 60, 60, cost=500)
        solde = rl.peek("c", 60, 60)
        assert solde is not None
        self.assertLess(solde, -400.0)

    def test_peek_none_si_illimite(self):
        rl = peppol_api.RateLimiter()
        self.assertIsNone(rl.peek("c", 0, 1))

    def test_retry_for_zero_si_jeton_dispo(self):
        # 1 jeton suffit à passer (seuil d'admission) : aucune attente.
        self.assertEqual(peppol_api.RateLimiter.retry_for(1.0, 60), 0.0)
        self.assertEqual(peppol_api.RateLimiter.retry_for(9.0, 60), 0.0)

    def test_retry_for_compte_la_remontee_a_un_jeton(self):
        # 60 req/min = 1 jeton/s ; à 0.5 jeton il manque 0.5 → 0.5 s.
        self.assertAlmostEqual(peppol_api.RateLimiter.retry_for(0.5, 60), 0.5)
        # En découvert de 440, il faut 441 s pour revenir à 1 jeton.
        self.assertAlmostEqual(peppol_api.RateLimiter.retry_for(-440.0, 60), 441.0)

    def test_retry_for_zero_si_illimite(self):
        self.assertEqual(peppol_api.RateLimiter.retry_for(-999.0, 0), 0.0)

    def test_retry_for_est_la_formule_du_429(self):
        """La valeur exposée par /limits doit être celle que le 429 renverrait —
        sinon un client qui dort `retry_after` se reprendrait un 429."""
        rl = peppol_api.RateLimiter()
        rl.allow("c", 60, 5, cost=5)              # seau vidé
        allowed, retry_du_429 = rl.allow("c", 60, 5)
        self.assertFalse(allowed)
        solde = rl.peek("c", 60, 5)
        assert solde is not None
        self.assertAlmostEqual(peppol_api.RateLimiter.retry_for(solde, 60),
                               retry_du_429, delta=0.05)


class _Srv(ThreadingHTTPServer):
    daemon_threads = True
    allow_reuse_address = True


class EndToEndTests(unittest.TestCase):
    """Serveur réel sur un port éphémère, résolveur monkeypatché."""

    @classmethod
    def setUpClass(cls):
        peppol_api.API_KEYS = {
            "testkey": peppol_api.KeyConfig("test", 0, 1),      # illimité
            "slowkey": peppol_api.KeyConfig("slow", 1, 1),      # 1 req/min, burst 1
            "tinykey": peppol_api.KeyConfig("tiny", 1, 1),      # idem, seau dédié
            "costkey": peppol_api.KeyConfig("cost", 60, 10),    # 1 jeton/s, burst 10
            "limitkey": peppol_api.KeyConfig("limit", 60, 10),  # pour /limits
            "peekkey": peppol_api.KeyConfig("peek", 1, 1),      # 1 seul jeton
            "drykey": peppol_api.KeyConfig("dry", 1, 1),        # idem, à épuiser
        }
        peppol_api._RL = peppol_api.RateLimiter()
        peppol_api._SEM = threading.BoundedSemaphore(4)

        def fake_resolve(participant, test):
            if "bad" in participant:
                raise ValueError("Cannot parse participant identifier")
            if "dnsfail" in participant:
                return RESULT_DNS_FAIL
            return RESULT_OK_EXT
        cls._orig = peppol_api.do_resolve
        peppol_api.do_resolve = fake_resolve

        cls.srv = _Srv(("127.0.0.1", 0), peppol_api.Handler)
        cls.port = cls.srv.server_address[1]
        cls.t = threading.Thread(target=cls.srv.serve_forever, daemon=True)
        cls.t.start()

    @classmethod
    def tearDownClass(cls):
        cls.srv.shutdown()
        cls.srv.server_close()
        peppol_api.do_resolve = cls._orig

    def _get(self, path, headers=None):
        url = f"http://127.0.0.1:{self.port}{path}"
        req = urllib.request.Request(url, headers=headers or {})
        try:
            with urllib.request.urlopen(req, timeout=5) as r:
                return r.status, json.loads(r.read().decode())
        except urllib.error.HTTPError as e:
            body = e.read().decode()
            try:
                return e.code, json.loads(body)
            except Exception:
                return e.code, body

    def _post(self, path, payload, headers=None, raw=False):
        url = f"http://127.0.0.1:{self.port}{path}"
        data = payload if raw else json.dumps(payload).encode()
        h = {"Content-Type": "application/json"}
        h.update(headers or {})
        req = urllib.request.Request(url, data=data, headers=h, method="POST")
        try:
            with urllib.request.urlopen(req, timeout=10) as r:
                return r.status, json.loads(r.read().decode())
        except urllib.error.HTTPError as e:
            body = e.read().decode()
            try:
                return e.code, json.loads(body)
            except Exception:
                return e.code, body

    def test_health_public(self):
        code, body = self._get("/health")
        self.assertEqual(code, 200)
        self.assertEqual(body["status"], "ok")

    def test_resolve_echec_dns_repond_503_pas_exists_false(self):
        # Annuaire momentanément inaccessible : le client doit voir une erreur
        # re-tentable, jamais un verdict « absent de Peppol ».
        code, body = self._get("/resolve/dnsfail", {"X-API-Key": "testkey"})
        self.assertEqual(code, 503)
        self.assertIn("SML", body["error"])

    def test_batch_echec_dns_est_une_erreur_par_entree(self):
        # Dans un lot, l'échec de lookup d'une entrée devient {participant,
        # error} (re-tentable), et ne contamine pas les autres entrées.
        code, body = self._post(
            "/resolve/batch",
            {"participants": ["0225:ext", "dnsfail"]},
            {"X-API-Key": "testkey"},
        )
        self.assertEqual(code, 200)
        self.assertTrue(body["results"][0]["exists"])
        self.assertIn("error", body["results"][1])
        self.assertIn("SML", body["results"][1]["error"])
        self.assertNotIn("exists", body["results"][1])

    def test_openapi_public(self):
        code, body = self._get("/openapi.json")
        self.assertEqual(code, 200)
        self.assertEqual(body["openapi"], "3.0.3")

    def test_resolve_requires_key(self):
        code, body = self._get("/resolve/0225:ext")
        self.assertEqual(code, 401)

    def test_resolve_with_key(self):
        code, body = self._get("/resolve/0225:ext", {"X-API-Key": "testkey"})
        self.assertEqual(code, 200)
        self.assertTrue(body["exists"])
        self.assertTrue(body["supports_extended_ctc_fr"])
        self.assertEqual(body["pa"]["code"], "PFR000123")

    def test_resolve_bearer(self):
        code, body = self._get("/resolve/0225:ext", {"Authorization": "Bearer testkey"})
        self.assertEqual(code, 200)

    def test_resolve_bad_participant(self):
        code, body = self._get("/resolve/bad", {"X-API-Key": "testkey"})
        self.assertEqual(code, 400)

    def test_resolve_query_param(self):
        code, body = self._get("/resolve?participant=0225:ext", {"X-API-Key": "testkey"})
        self.assertEqual(code, 200)

    def test_resolve_detail_full(self):
        code, body = self._get("/resolve/0225:ext?detail=full", {"X-API-Key": "testkey"})
        self.assertEqual(code, 200)
        self.assertIn("detail", body)
        self.assertIn("access_points", body["detail"])

    def test_per_key_rate_limit_429(self):
        # 'slowkey' : 1 req/min, burst 1 -> 1re OK, 2e 429 (résolveur factice
        # instantané, donc pas de recharge entre les deux appels).
        c1, _ = self._get("/resolve/0225:ext", {"X-API-Key": "slowkey"})
        c2, body = self._get("/resolve/0225:ext", {"X-API-Key": "slowkey"})
        self.assertEqual(c1, 200)
        self.assertEqual(c2, 429)

    def test_resolve_participant_too_long(self):
        code, body = self._get("/resolve/" + "0" * 300, {"X-API-Key": "testkey"})
        self.assertEqual(code, 400)
        self.assertIn("trop long", body["error"])

    def test_unknown_route(self):
        code, body = self._get("/nope", {"X-API-Key": "testkey"})
        self.assertEqual(code, 404)

    # --- /resolve/batch ----------------------------------------------------
    def test_batch_requires_key(self):
        code, _ = self._post("/resolve/batch", {"participants": ["0225:ext"]})
        self.assertEqual(code, 401)

    def test_batch_ok(self):
        code, body = self._post(
            "/resolve/batch", {"participants": ["0225:ext", "0225:ext2"]},
            {"X-API-Key": "testkey"})
        self.assertEqual(code, 200)
        self.assertEqual(body["count"], 2)
        self.assertTrue(body["results"][0]["exists"])
        self.assertTrue(body["results"][1]["exists"])

    def test_batch_mixed_error_isolated(self):
        # 'bad' fait lever ValueError côté résolveur factice -> item en erreur,
        # l'autre reste OK (une entrée en échec ne casse pas le lot).
        code, body = self._post(
            "/resolve/batch", {"participants": ["0225:ok", "bad"]},
            {"X-API-Key": "testkey"})
        self.assertEqual(code, 200)
        self.assertTrue(body["results"][0]["exists"])
        self.assertIn("error", body["results"][1])

    def test_batch_order_and_dupes_preserved(self):
        code, body = self._post(
            "/resolve/batch", {"participants": ["0225:a", "0225:a", "0225:b"]},
            {"X-API-Key": "testkey"})
        self.assertEqual(code, 200)
        self.assertEqual(body["count"], 3)

    def test_batch_at_max_ok(self):
        code, body = self._post(
            "/resolve/batch",
            {"participants": [f"0225:{i}" for i in range(peppol_api.BATCH_MAX)]},
            {"X-API-Key": "testkey"})
        self.assertEqual(code, 200)
        self.assertEqual(body["count"], peppol_api.BATCH_MAX)

    def test_batch_too_large(self):
        code, body = self._post(
            "/resolve/batch",
            {"participants": [f"0225:{i}" for i in range(peppol_api.BATCH_MAX + 1)]},
            {"X-API-Key": "testkey"})
        self.assertEqual(code, 400)
        self.assertIn("trop grand", body["error"])

    def test_batch_too_large_costs_no_token(self):
        """Le contrôle de taille précède le rate-limit : un batch invalide est
        gratuit. 'tinykey' n'a qu'un jeton — s'il avait été prélevé par le 400,
        la requête suivante répondrait 429."""
        code, body = self._post(
            "/resolve/batch",
            {"participants": [f"0225:{i}" for i in range(peppol_api.BATCH_MAX + 1)]},
            {"X-API-Key": "tinykey"})
        self.assertEqual(code, 400)
        self.assertIn("trop grand", body["error"])
        code, _ = self._get("/resolve/0225:ext", {"X-API-Key": "tinykey"})
        self.assertEqual(code, 200)

    def test_batch_costs_one_token_per_participant(self):
        """Un batch de 4 coûte 4 jetons, pas 1. Seau de 10 : deux batches le
        vident, le troisième passe en découvert, le quatrième est refusé.
        À 1 jeton par requête, les dix premiers passeraient."""
        participants = [f"0225:{i}" for i in range(4)]
        for _ in range(3):
            code, _ = self._post("/resolve/batch", {"participants": participants},
                                 {"X-API-Key": "costkey"})
            self.assertEqual(code, 200)
        code, _ = self._post("/resolve/batch", {"participants": participants},
                             {"X-API-Key": "costkey"})
        self.assertEqual(code, 429)

    def test_batch_empty_list(self):
        code, _ = self._post("/resolve/batch", {"participants": []}, {"X-API-Key": "testkey"})
        self.assertEqual(code, 400)

    def test_batch_not_object(self):
        code, _ = self._post("/resolve/batch", ["x"], {"X-API-Key": "testkey"})
        self.assertEqual(code, 400)

    def test_batch_invalid_json(self):
        code, _ = self._post("/resolve/batch", b"{not json", {"X-API-Key": "testkey"}, raw=True)
        self.assertEqual(code, 400)

    def test_get_on_batch_405(self):
        code, _ = self._get("/resolve/batch", {"X-API-Key": "testkey"})
        self.assertEqual(code, 405)

    # --- /limits -----------------------------------------------------------
    def test_limits_requires_key(self):
        code, _ = self._get("/limits")
        self.assertEqual(code, 401)

    def test_limits_expose_limit_et_burst_de_la_cle(self):
        code, body = self._get("/limits", {"X-API-Key": "limitkey"})
        self.assertEqual(code, 200)
        self.assertEqual(body["limit"], 60)          # req/min de 'limitkey'
        self.assertEqual(body["burst"], 10)
        self.assertAlmostEqual(body["current"], 10.0, delta=0.5)   # seau neuf

    def test_limits_ne_consomme_pas_de_jeton(self):
        """'peekkey' n'a qu'un jeton. Le consulter trois fois ne doit pas le
        dépenser : sinon consulter ses limites suffirait à les épuiser."""
        for _ in range(3):
            code, body = self._get("/limits", {"X-API-Key": "peekkey"})
            self.assertEqual(code, 200)
            self.assertAlmostEqual(body["current"], 1.0, delta=0.1)
        code, _ = self._get("/resolve/0225:ext", {"X-API-Key": "peekkey"})
        self.assertEqual(code, 200)                  # le jeton était bien intact

    def test_limits_reflete_le_cout_d_un_batch(self):
        """Après un batch de 4, le solde a baissé de 4 — c'est ce qui rend
        l'endpoint utile : le client sait ce qu'il lui reste."""
        _, avant = self._get("/limits", {"X-API-Key": "limitkey"})
        code, _ = self._post("/resolve/batch",
                             {"participants": [f"0225:{i}" for i in range(4)]},
                             {"X-API-Key": "limitkey"})
        self.assertEqual(code, 200)
        _, apres = self._get("/limits", {"X-API-Key": "limitkey"})
        self.assertAlmostEqual(avant["current"] - apres["current"], 4.0, delta=0.5)

    def test_limits_cle_illimitee(self):
        """'testkey' a rate 0 (illimité) : pas de seau, donc pas de solde, et
        jamais d'attente."""
        code, body = self._get("/limits", {"X-API-Key": "testkey"})
        self.assertEqual(code, 200)
        self.assertEqual(body["limit"], 0)
        self.assertIsNone(body["current"])
        self.assertEqual(body["retry_after"], 0)

    def test_limits_retry_after_nul_quand_le_quota_est_dispo(self):
        code, body = self._get("/limits", {"X-API-Key": "limitkey"})
        self.assertEqual(code, 200)
        self.assertEqual(body["retry_after"], 0)

    def test_limits_retry_after_quand_le_quota_est_epuise(self):
        """'drykey' : 1 req/min, burst 1. Une fois son jeton dépensé, /limits
        doit dire combien de temps attendre — c'est tout l'intérêt du champ."""
        code, _ = self._get("/resolve/0225:ext", {"X-API-Key": "drykey"})
        self.assertEqual(code, 200)                       # le seul jeton part ici
        code, body = self._get("/limits", {"X-API-Key": "drykey"})
        self.assertEqual(code, 200)
        self.assertLess(body["current"], 1.0)
        # 1 req/min = 1 jeton/60 s : il faut ~60 s pour en regagner un.
        self.assertGreater(body["retry_after"], 55.0)
        self.assertLessEqual(body["retry_after"], 60.0)


class BatchUnitTests(unittest.TestCase):
    def setUp(self):
        peppol_api._SEM = threading.BoundedSemaphore(4)

    def test_dedup_resolves_each_once(self):
        calls = []
        orig = peppol_api.do_resolve
        peppol_api.do_resolve = lambda p, t: (calls.append(p), RESULT_OK_EXT)[1]
        try:
            res = peppol_api.resolve_batch(["0225:a", "0225:a", "0225:b"], False)
        finally:
            peppol_api.do_resolve = orig
        self.assertEqual(len(res), 3)                       # entrées/ordre préservés
        self.assertEqual(sorted(calls), ["0225:a", "0225:b"])  # résolu une fois chacun


if __name__ == "__main__":
    unittest.main()
