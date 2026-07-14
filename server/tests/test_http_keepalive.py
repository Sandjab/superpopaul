"""http_get : keep-alive par hôte (pool thread-local, sans proxy).

Le profilage du 2026-07-14 (tools/profil_resolution.py) impute ~58 % du CPU
d'une résolution à l'établissement des connexions (handshake TLS +
getaddrinfo + connect, un par requête : urllib n'a pas de keep-alive).
Réutiliser la connexion par hôte SMP supprime ce coût — ces tests vérifient
la réutilisation ET que le contrat d'erreur urllib est préservé (le retry
429/5xx de http_get et les appelants reposent sur urllib.error.HTTPError).
"""
import http.server
import threading
import unittest
import urllib.error

import peppol_resolver


class _Serveur(http.server.ThreadingHTTPServer):
    connexions: int = 0  # sockets TCP acceptées


class _Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"  # keep-alive côté serveur

    def setup(self):
        super().setup()
        assert isinstance(self.server, _Serveur)
        self.server.connexions += 1  # une par socket TCP acceptée

    def _body(self, code: int, body: bytes, close: bool = False):
        self.send_response(code)
        self.send_header("Content-Length", str(len(body)))
        if close:
            self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(body)
        if close:
            self.close_connection = True

    def do_GET(self):
        if self.path == "/ferme":
            self._body(200, b"ferme", close=True)
        elif self.path == "/absent":
            self._body(404, b"pas ici")
        elif self.path == "/redirige":
            self.send_response(302)
            self.send_header("Location", "/cible")
            self.send_header("Content-Length", "0")
            self.end_headers()
        else:
            self._body(200, b"ok:" + self.path.encode())

    def log_message(self, format, *args):
        pass  # silence


class HttpKeepAliveTests(unittest.TestCase):
    def setUp(self):
        self.srv = _Serveur(("127.0.0.1", 0), _Handler)
        self.srv.connexions = 0
        threading.Thread(target=self.srv.serve_forever, daemon=True).start()
        self.base = f"http://127.0.0.1:{self.srv.server_address[1]}"
        # Réinitialise réseau ET pool (l'époque du pool invalide les
        # connexions des configurations précédentes).
        peppol_resolver.configure_network()

    def tearDown(self):
        self.srv.shutdown()
        self.srv.server_close()

    def test_connexion_reutilisee_sur_le_meme_hote(self):
        # Trois GET vers le même hôte = une seule connexion TCP, sinon on
        # repaie handshake + getaddrinfo à chaque requête (~58 % du CPU).
        for i in range(3):
            self.assertIn("ok:", peppol_resolver.http_get(f"{self.base}/p{i}"))
        self.assertEqual(self.srv.connexions, 1)

    def test_connexion_fermee_par_le_serveur_reprise(self):
        # Un serveur peut fermer entre deux requêtes (Connection: close,
        # timeout keep-alive) : la requête suivante doit rouvrir en silence,
        # jamais remonter une erreur à l'appelant.
        self.assertEqual(peppol_resolver.http_get(f"{self.base}/ferme"), "ferme")
        self.assertIn("ok:", peppol_resolver.http_get(f"{self.base}/apres"))
        self.assertEqual(self.srv.connexions, 2)

    def test_erreur_http_compatible_urllib(self):
        # Le retry 429/5xx de http_get et resolve() attrapent
        # urllib.error.HTTPError (code, headers, read) : le chemin poolé
        # doit lever la même chose, pas une http.client.HTTPException.
        with self.assertRaises(urllib.error.HTTPError) as cm:
            peppol_resolver.http_get(f"{self.base}/absent")
        self.assertEqual(cm.exception.code, 404)
        self.assertEqual(cm.exception.read(), b"pas ici")

    def test_redirection_suivie(self):
        # urllib suivait les redirections ; le pool doit les suivre aussi
        # (SMP derrière un 30x, constaté sur des hébergeurs mutualisés).
        self.assertEqual(peppol_resolver.http_get(f"{self.base}/redirige"),
                         "ok:/cible")

    def test_proxy_desactive_le_pool(self):
        # Derrière un proxy on garde le chemin urllib (CONNECT, auth) tel
        # quel : le pool ne doit pas s'y substituer.
        peppol_resolver.configure_network(proxy="http://127.0.0.1:1")
        try:
            self.assertFalse(peppol_resolver._pool_enabled())
        finally:
            peppol_resolver.configure_network()

    def test_configure_network_invalide_le_pool(self):
        # Reconfigurer le réseau (CA, proxy…) doit jeter les connexions
        # ouvertes : elles portent l'ancien contexte TLS.
        self.assertIn("ok:", peppol_resolver.http_get(f"{self.base}/a"))
        peppol_resolver.configure_network()
        self.assertIn("ok:", peppol_resolver.http_get(f"{self.base}/b"))
        self.assertEqual(self.srv.connexions, 2)


if __name__ == "__main__":
    unittest.main()
