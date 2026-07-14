"""popaul : canonicalisation des adressages — parité avec le client
graphique (client/src-tauri/src/pid.rs).

Les cas sont le miroir exact des tests Rust `pid::tests` : toute évolution
d'un côté doit être reportée de l'autre.
"""
import unittest

from popaul import canonical


class TestCanonical(unittest.TestCase):
    def test_ajoute_le_scheme_par_defaut(self):
        self.assertEqual(canonical("0009:552100554"),
                         "iso6523-actorid-upis::0009:552100554")

    def test_conserve_un_pid_deja_complet(self):
        self.assertEqual(canonical("iso6523-actorid-upis::0009:552100554"),
                         "iso6523-actorid-upis::0009:552100554")

    def test_trimme(self):
        self.assertEqual(canonical("  0009:1  "), "iso6523-actorid-upis::0009:1")

    def test_prefixe_0225_sur_un_adressage_brut_sans_icd(self):
        # Règle métier : l'adressage brut (SIREN, SIREN_SIRET,
        # SIREN_SIRET_CODEROUTAGE, SIREN_SUFFIXELIBRE — jamais de « : »)
        # se canonicalise TOUJOURS en iso6523-actorid-upis::0225:<brut>.
        # Sans l'ICD, le hash SML porte sur la valeur nue → faux négatif
        # systématique (« absent de Peppol » pour un inscrit).
        for brut in ("552100554", "552100554_55210055400013",
                     "552100554_55210055400013_ROUTAGE1", "552100554_SERVICE_ACHATS"):
            self.assertEqual(canonical(brut), f"iso6523-actorid-upis::0225:{brut}")

    def test_respecte_un_icd_explicite(self):
        # Un « : » simple signale un ICD déjà présent : on n'empile pas 0225.
        self.assertEqual(canonical("0225:552100554"),
                         "iso6523-actorid-upis::0225:552100554")
        self.assertEqual(canonical("0009:552100554"),
                         "iso6523-actorid-upis::0009:552100554")


if __name__ == "__main__":
    unittest.main()
