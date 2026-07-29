# Résolution d'un adressage unitaire — plan d'implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** une loupe dans l'en-tête ouvre une modale qui résout un adressage saisi contre le réseau Peppol, l'annuaire Peppol et l'annuaire PPF, et restitue les champs de l'export — sans rien écrire en base.

**Architecture:** un module pur `unitaire.rs` porte les verdicts (les trois indéterminations, l'assemblage des champs réseau) et se teste sans réseau ni UI. `commands.rs` n'ajoute qu'un câblage, sur le modèle de `calculer_rapprochement` (fonction interne prenant `&Arc<Mutex<Store>>`, testable avec `Store::open_in_memory` + wiremock). La chaîne réseau est celle d'un run — `pid::canonical` → client courant → `resolver::to_resolution` → `output::ctc_status` — pour que l'écran montre ce que l'export produirait.

**Tech Stack:** Rust (Tauri 2, rusqlite, wiremock, tokio), JS vanilla (aucun bundler), tests `node --test` avec le faux DOM `client/tests/dom_shim.js`.

Spec : `docs/superpowers/specs/2026-07-29-resolution-unitaire-design.md`
Maquette : `docs/superpowers/maquettes/2026-07-29-resolution-unitaire.html`

---

## Structure des fichiers

| Fichier | Rôle |
|---|---|
| `client/src-tauri/src/unitaire.rs` | **Créé.** Types de retour et verdicts purs : `Muette`, `Annuaire`, `Ppf`, `ChampsReseau`, `ResolutionUnitaire`. Aucune I/O. |
| `client/src-tauri/src/lib.rs` | **Modifié.** Déclare `mod unitaire;` et enregistre la commande. |
| `client/src-tauri/src/resolver.rs` | **Modifié.** `to_resolution` passe de privé à `pub(crate)`. |
| `client/src-tauri/src/commands.rs` | **Modifié.** `resoudre_adressage_impl` (testable) + `resoudre_adressage` (commande Tauri). |
| `client/src/index.html` | **Modifié.** Bouton `#btn-resolve` dans `.cfg-btns`. |
| `client/src/app.js` | **Modifié.** Ouverture de la modale, appel, rendu. |
| `client/src/styles.css` | **Modifié.** Styles des sections, des champs de vedette et des couleurs de verdict. |
| `client/tests/resolution_unitaire.test.js` | **Créé.** Câblage de la modale. |
| `client/tests/legende_parite.test.js` | **Créé.** Les infobulles ne divergent pas de `docs/legende_champs.md`. |

---

### Task 1 : les indéterminations de l'annuaire Peppol

**Files:**
- Create: `client/src-tauri/src/unitaire.rs`
- Modify: `client/src-tauri/src/lib.rs`

- [ ] **Step 1 : déclarer le module**

Dans `client/src-tauri/src/lib.rs`, ajouter à la liste des `mod` (ordre alphabétique parmi les autres) :

```rust
mod unitaire;
```

- [ ] **Step 2 : écrire le test qui échoue**

Créer `client/src-tauri/src/unitaire.rs` avec **uniquement** ce contenu :

```rust
//! Verdicts de la résolution unitaire (loupe de l'en-tête). Module PUR :
//! aucune I/O, aucun accès base — l'appelant fournit ce qu'il a lu.
//!
//! Règle qui gouverne tout le module : une source qui ne peut pas répondre ne
//! répond JAMAIS `false`. « Je ne sais pas » et « non » sont deux réponses
//! différentes, et les confondre ferait lire un constat rassurant là où il n'y
//! en a pas (même discipline que `avertissement_ppf_cumulatif`).

use serde::Serialize;

/// Pourquoi une source reste muette.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Muette {
    /// L'annuaire Peppol n'a jamais été chargé.
    AnnuaireNonCharge,
    /// L'annuaire PPF est vide.
    AnnuaireVide,
    /// L'adressage n'est pas un 0225 : les deux annuaires sont indexés sur la
    /// valeur nue 0225, un autre ICD n'y est pas « absent », il n'y est pas
    /// cherchable.
    HorsPerimetre0225,
}

/// Ce que l'annuaire Peppol a à dire. Répond OU se tait, jamais les deux.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "etat", rename_all = "snake_case")]
pub enum Annuaire {
    Repond { in_directory: bool },
    Muette { raison: Muette },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hors_0225_prime_sur_l_annuaire_non_charge() {
        // Les deux causes peuvent coexister. On nomme la STRUCTURELLE : charger
        // l'annuaire ne rendrait pas cet adressage cherchable pour autant.
        assert_eq!(
            etat_annuaire_peppol(None, false, false),
            Annuaire::Muette { raison: Muette::HorsPerimetre0225 }
        );
    }

    #[test]
    fn annuaire_jamais_charge_ne_dit_pas_false() {
        assert_eq!(
            etat_annuaire_peppol(Some("552100554"), false, false),
            Annuaire::Muette { raison: Muette::AnnuaireNonCharge }
        );
    }

    #[test]
    fn annuaire_charge_rend_la_presence() {
        assert_eq!(
            etat_annuaire_peppol(Some("552100554"), true, true),
            Annuaire::Repond { in_directory: true }
        );
        assert_eq!(
            etat_annuaire_peppol(Some("552100554"), true, false),
            Annuaire::Repond { in_directory: false }
        );
    }
}
```

- [ ] **Step 3 : lancer le test pour le voir échouer**

Run : `cd client/src-tauri && cargo test unitaire::`
Expected : FAIL — `cannot find function 'etat_annuaire_peppol' in this scope`.

- [ ] **Step 4 : écrire l'implémentation minimale**

Ajouter dans `unitaire.rs`, avant `mod tests` :

```rust
/// Verdict de l'annuaire Peppol. `valeur_0225` vient de
/// `directory::parse_0225_value` (None = autre ICD) ; `charge` de
/// `store::peppol_directory_status().is_some()` ; `present` de
/// `store::directory_present`.
pub fn etat_annuaire_peppol(valeur_0225: Option<&str>, charge: bool, present: bool) -> Annuaire {
    match (valeur_0225, charge) {
        (None, _) => Annuaire::Muette { raison: Muette::HorsPerimetre0225 },
        (Some(_), false) => Annuaire::Muette { raison: Muette::AnnuaireNonCharge },
        (Some(_), true) => Annuaire::Repond { in_directory: present },
    }
}
```

- [ ] **Step 5 : lancer le test pour le voir passer**

Run : `cd client/src-tauri && cargo test unitaire::`
Expected : PASS — 3 tests.

- [ ] **Step 6 : commit**

```bash
git add client/src-tauri/src/unitaire.rs client/src-tauri/src/lib.rs
git commit -m "feat(superpopaul): verdict de l'annuaire Peppol pour la résolution unitaire"
```

---

### Task 2 : les indéterminations de l'annuaire PPF

**Files:**
- Modify: `client/src-tauri/src/unitaire.rs`

- [ ] **Step 1 : écrire le test qui échoue**

Ajouter dans `mod tests` de `unitaire.rs` :

```rust
    use crate::store::PpfFlags;

    fn flags(in_ppf: bool, active: bool, pdp_definie: bool, usable: bool) -> PpfFlags {
        PpfFlags { in_ppf, active, pdp_definie, usable }
    }

    #[test]
    fn ppf_hors_0225_est_muet() {
        assert_eq!(
            etat_ppf(None, true, None),
            Ppf::Muette { raison: Muette::HorsPerimetre0225 }
        );
    }

    #[test]
    fn ppf_vide_ne_dit_pas_false() {
        assert_eq!(
            etat_ppf(Some("552100554"), false, None),
            Ppf::Muette { raison: Muette::AnnuaireVide }
        );
    }

    #[test]
    fn absent_d_un_annuaire_charge_est_un_vrai_non() {
        // `ppf_flags` ne rend une entrée QUE pour les identifiants trouvés :
        // absent de la map, annuaire non vide = il n'y est pas, pour de bon.
        assert_eq!(
            etat_ppf(Some("552100554"), true, None),
            Ppf::Repond {
                annuaire_ppf: false,
                ppf_active: false,
                pdp_definie: false,
                ppf_usable: false,
            }
        );
    }

    #[test]
    fn les_quatre_drapeaux_sont_recopies_sans_recalcul() {
        // ppf_usable ne se déduit pas de active && pdp_definie : le store exige
        // les deux sur la MÊME ligne. Recalculer ici inventerait un `true`.
        assert_eq!(
            etat_ppf(Some("x"), true, Some(&flags(true, true, true, false))),
            Ppf::Repond {
                annuaire_ppf: true,
                ppf_active: true,
                pdp_definie: true,
                ppf_usable: false,
            }
        );
    }
```

Et le type, avant `mod tests` :

```rust
/// Ce que l'annuaire PPF a à dire. Les quatre drapeaux sont ceux de
/// `store::ppf_flags`, recopiés tels quels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "etat", rename_all = "snake_case")]
pub enum Ppf {
    Repond {
        annuaire_ppf: bool,
        ppf_active: bool,
        pdp_definie: bool,
        ppf_usable: bool,
    },
    Muette { raison: Muette },
}
```

- [ ] **Step 2 : lancer le test pour le voir échouer**

Run : `cd client/src-tauri && cargo test unitaire::`
Expected : FAIL — `cannot find function 'etat_ppf' in this scope`.

- [ ] **Step 3 : écrire l'implémentation minimale**

```rust
/// Verdict de l'annuaire PPF. `non_vide` vient de
/// `store::ppf_summary().distinct_addr > 0` ; `flags` de `store::ppf_flags`,
/// qui n'a d'entrée que pour les identifiants trouvés.
pub fn etat_ppf(
    valeur_0225: Option<&str>,
    non_vide: bool,
    flags: Option<&crate::store::PpfFlags>,
) -> Ppf {
    match (valeur_0225, non_vide) {
        (None, _) => Ppf::Muette { raison: Muette::HorsPerimetre0225 },
        (Some(_), false) => Ppf::Muette { raison: Muette::AnnuaireVide },
        (Some(_), true) => match flags {
            Some(f) => Ppf::Repond {
                annuaire_ppf: f.in_ppf,
                ppf_active: f.active,
                pdp_definie: f.pdp_definie,
                ppf_usable: f.usable,
            },
            None => Ppf::Repond {
                annuaire_ppf: false,
                ppf_active: false,
                pdp_definie: false,
                ppf_usable: false,
            },
        },
    }
}
```

- [ ] **Step 4 : lancer le test pour le voir passer**

Run : `cd client/src-tauri && cargo test unitaire::`
Expected : PASS — 7 tests.

- [ ] **Step 5 : commit**

```bash
git add client/src-tauri/src/unitaire.rs
git commit -m "feat(superpopaul): verdict de l'annuaire PPF pour la résolution unitaire"
```

---

### Task 3 : les champs réseau, avec l'état CTC de l'export

**Files:**
- Modify: `client/src-tauri/src/unitaire.rs`
- Modify: `client/src-tauri/src/resolver.rs:250`

- [ ] **Step 1 : écrire le test qui échoue**

Ajouter dans `mod tests` :

```rust
    use chrono::{TimeZone, Utc};
    use crate::store::Resolution;

    fn resolution(activation: Option<&str>, expiration: Option<&str>, ctc: Option<bool>) -> Resolution {
        Resolution {
            participant: "iso6523-actorid-upis::0225:552100554".into(),
            exists_in_peppol: Some(true),
            pa_code: Some("PA0042".into()),
            pa_name: Some("ACME Services".into()),
            pa_country: Some("FR".into()),
            extended_ctc_fr: ctc,
            api_status: "ok".into(),
            resolved_at: 0,
            note: None,
            ctc_activation: activation.map(str::to_string),
            ctc_expiration: expiration.map(str::to_string),
        }
    }

    #[test]
    fn l_etat_ctc_est_celui_de_l_export_pour_les_quatre_cas() {
        // Ces quatre valeurs SONT la colonne ctc_status du CSV. Les recalculer
        // ici (par exemple « activation passée ⇒ ready » sans regarder
        // ubl_extended) ferait diverger l'écran de l'export qu'il prétend
        // montrer.
        let now = Utc.with_ymd_and_hms(2026, 7, 29, 12, 0, 0).unwrap();
        assert_eq!(champs_reseau(&resolution(None, None, Some(true)), now).ctc_status, "ready");
        assert_eq!(
            champs_reseau(&resolution(Some("2030-01-01T00:00:00Z"), None, Some(true)), now).ctc_status,
            "later"
        );
        assert_eq!(
            champs_reseau(&resolution(None, Some("2020-01-01T00:00:00Z"), Some(true)), now).ctc_status,
            "expired"
        );
        // Sans déclaration CTC-FR, il n'y a aucun état à calculer.
        assert_eq!(champs_reseau(&resolution(None, None, Some(false)), now).ctc_status, "");
    }

    #[test]
    fn les_champs_du_pa_sont_recopies() {
        let now = Utc.with_ymd_and_hms(2026, 7, 29, 12, 0, 0).unwrap();
        let c = champs_reseau(&resolution(None, None, Some(true)), now);
        assert_eq!(c.in_peppol, Some(true));
        assert_eq!(c.pa_code.as_deref(), Some("PA0042"));
        assert_eq!(c.pa_country.as_deref(), Some("FR"));
        assert_eq!(c.ubl_extended, Some(true));
    }
```

- [ ] **Step 2 : lancer le test pour le voir échouer**

Run : `cd client/src-tauri && cargo test unitaire::`
Expected : FAIL — `cannot find function 'champs_reseau' in this scope`.

- [ ] **Step 3 : écrire l'implémentation minimale**

Dans `unitaire.rs`, avant `mod tests` :

```rust
/// Les huit champs Peppol de l'export, plus la note diagnostique du résolveur
/// (« ServiceGroup HTTP 403 on … » quand le catalogue SMP est illisible).
/// Les noms sont ceux de `output::field_name` : l'écran ne doit pas inventer un
/// vocabulaire parallèle.
#[derive(Debug, Clone, Serialize)]
pub struct ChampsReseau {
    pub in_peppol: Option<bool>,
    pub pa_code: Option<String>,
    pub pa_name: Option<String>,
    pub pa_country: Option<String>,
    pub ubl_extended: Option<bool>,
    pub ctc_activation: Option<String>,
    pub ctc_expiration: Option<String>,
    /// « ready » | « later » | « expired » | «  » — TOUJOURS via
    /// `output::ctc_status`, jamais recalculé ici.
    pub ctc_status: String,
    pub note: Option<String>,
}

/// Traduit une résolution (celle qu'un run écrirait) en champs d'affichage.
pub fn champs_reseau(r: &crate::store::Resolution, now: chrono::DateTime<chrono::Utc>) -> ChampsReseau {
    ChampsReseau {
        in_peppol: r.exists_in_peppol,
        pa_code: r.pa_code.clone(),
        pa_name: r.pa_name.clone(),
        pa_country: r.pa_country.clone(),
        ubl_extended: r.extended_ctc_fr,
        ctc_activation: r.ctc_activation.clone(),
        ctc_expiration: r.ctc_expiration.clone(),
        ctc_status: crate::output::ctc_status(r, now).to_string(),
        note: r.note.clone(),
    }
}
```

- [ ] **Step 4 : lancer le test pour le voir passer**

Run : `cd client/src-tauri && cargo test unitaire::`
Expected : PASS — 9 tests.

- [ ] **Step 5 : ouvrir `to_resolution` au crate**

Dans `client/src-tauri/src/resolver.rs`, ligne 250, remplacer :

```rust
fn to_resolution(item: &ApiItem, sent: &str, at: i64) -> Resolution {
```

par :

```rust
/// `pub(crate)` pour la résolution unitaire (`commands::resoudre_adressage`) :
/// elle DOIT produire la même Resolution qu'un run, sans quoi l'écran de la
/// loupe et le fichier de sortie pourraient diverger.
pub(crate) fn to_resolution(item: &ApiItem, sent: &str, at: i64) -> Resolution {
```

- [ ] **Step 6 : vérifier que rien n'est cassé**

Run : `cd client/src-tauri && cargo test`
Expected : PASS — 640 tests (631 + 9), 0 échec.

- [ ] **Step 7 : commit**

```bash
git add client/src-tauri/src/unitaire.rs client/src-tauri/src/resolver.rs
git commit -m "feat(superpopaul): champs réseau de la résolution unitaire, état CTC repris de l'export"
```

---

### Task 4 : la commande

**Files:**
- Modify: `client/src-tauri/src/unitaire.rs`
- Modify: `client/src-tauri/src/commands.rs`
- Modify: `client/src-tauri/src/lib.rs:75`

- [ ] **Step 1 : ajouter les types d'enveloppe**

Dans `unitaire.rs`, avant `mod tests` :

```rust
/// Ce que le réseau a répondu. Un échec n'est PAS une erreur de commande : les
/// annuaires locaux, eux, savent répondre sans réseau.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "etat", rename_all = "snake_case")]
pub enum Reseau {
    Repond { champs: ChampsReseau, latence_ms: u64 },
    Echec { message: String },
}

/// Réponse complète de la loupe.
#[derive(Debug, Clone, Serialize)]
pub struct ResolutionUnitaire {
    /// Tel que tapé (trim) — pour que l'écran puisse montrer l'écart avec la
    /// forme canonique.
    pub saisi: String,
    pub canonique: String,
    /// « api » | « direct » : le verdict n'est comparable à celui d'un run que
    /// si le transport est le même, autant le dire.
    pub mode: String,
    pub reseau: Reseau,
    pub annuaire_peppol: Annuaire,
    pub ppf: Ppf,
}
```

- [ ] **Step 2 : écrire le test qui échoue**

Dans `client/src-tauri/src/commands.rs`, dans `mod tests`, ajouter :

```rust
    #[tokio::test]
    async fn la_loupe_envoie_la_forme_canonique_et_survit_a_un_echec_reseau() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        // Un SIREN nu doit partir en 0225 canonique : sans le préfixe, le hash
        // SML porterait sur la valeur nue et tout ressortirait « absent ».
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/resolve/batch"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": [{
                    "participant_id": "iso6523-actorid-upis::0225:552100554",
                    "exists": true, "supports_extended_ctc_fr": true
                }]
            })))
            .mount(&server)
            .await;

        let store = Arc::new(Mutex::new(Store::open_in_memory().expect("store en mémoire")));
        let client = crate::api::ApiClient::new(&server.uri(), "K", None, None).unwrap();
        let now = chrono::Utc::now();
        let r = resoudre_adressage_impl(&client, &store, &["C".into()], "api", "552100554", now)
            .await
            .expect("la commande doit aboutir");

        assert_eq!(r.canonique, "iso6523-actorid-upis::0225:552100554");
        assert!(matches!(r.reseau, crate::unitaire::Reseau::Repond { .. }));
        // Aucun annuaire chargé dans ce store neuf : muets, jamais « false ».
        assert!(matches!(
            r.annuaire_peppol,
            crate::unitaire::Annuaire::Muette { raison: crate::unitaire::Muette::AnnuaireNonCharge }
        ));
        assert!(matches!(
            r.ppf,
            crate::unitaire::Ppf::Muette { raison: crate::unitaire::Muette::AnnuaireVide }
        ));
    }

    #[tokio::test]
    async fn un_echec_reseau_laisse_la_commande_aboutir() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        // Le point : les annuaires ne dépendent pas du réseau. Rendre Err ici
        // priverait l'utilisateur d'informations que la machine possède.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/resolve/batch"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;

        let store = Arc::new(Mutex::new(Store::open_in_memory().expect("store en mémoire")));
        let client = crate::api::ApiClient::new(&server.uri(), "K", None, None).unwrap();
        let r = resoudre_adressage_impl(
            &client, &store, &["C".into()], "api", "552100554", chrono::Utc::now(),
        )
        .await
        .expect("un échec réseau ne doit pas faire échouer la commande");

        match r.reseau {
            crate::unitaire::Reseau::Echec { ref message } => {
                assert!(message.contains("navigateur"), "message inattendu : {message}");
            }
            other => panic!("attendu Echec, obtenu {other:?}"),
        }
    }

    #[tokio::test]
    async fn une_saisie_vide_est_refusee() {
        let store = Arc::new(Mutex::new(Store::open_in_memory().expect("store en mémoire")));
        let client = crate::api::ApiClient::new("http://127.0.0.1:1", "K", None, None).unwrap();
        let res = resoudre_adressage_impl(
            &client, &store, &["C".into()], "api", "   ", chrono::Utc::now(),
        )
        .await;
        assert!(res.is_err(), "une saisie vide ne doit pas partir sur le réseau");
    }
```

- [ ] **Step 3 : lancer le test pour le voir échouer**

Run : `cd client/src-tauri && cargo test resoudre_adressage`
Expected : FAIL — `cannot find function 'resoudre_adressage_impl' in this scope`.

- [ ] **Step 4 : écrire l'implémentation**

Dans `commands.rs`, à la suite de `test_api` :

```rust
/// Cœur testable de la loupe (même motif que `calculer_rapprochement`) : prend
/// ce dont il a besoin, pas un `State` — les commandes Tauri ne se fabriquent
/// pas en test.
///
/// N'ÉCRIT RIEN : consulter un adressage ne doit pas le retirer du périmètre
/// d'un run futur (les modes lisent `resolutions` pour savoir ce qui reste à
/// faire).
async fn resoudre_adressage_impl(
    client: &ApiClient,
    store: &Arc<Mutex<Store>>,
    motifs: &[String],
    mode: &str,
    saisi: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<crate::unitaire::ResolutionUnitaire, String> {
    let saisi = saisi.trim();
    if saisi.is_empty() {
        return Err("Saisissez un adressage.".into());
    }
    let canonique = crate::pid::canonical(saisi);
    let valeur_0225 = crate::directory::parse_0225_value(&canonique);

    // Annuaires d'abord : ils répondent même si le réseau ne répond pas.
    let (charge, present, ppf_non_vide, flags) = {
        let s = store.lock().unwrap();
        let charge = s.peppol_directory_status()?.is_some();
        let valeurs: Vec<String> = valeur_0225.iter().cloned().collect();
        let present = if charge && !valeurs.is_empty() {
            !s.directory_present(&valeurs)?.is_empty()
        } else {
            false
        };
        let ppf_non_vide = s.ppf_summary()?.distinct_addr > 0;
        let flags = if ppf_non_vide && !valeurs.is_empty() {
            s.ppf_flags(&valeurs, motifs)?
                .get(valeurs[0].as_str())
                .cloned()
        } else {
            None
        };
        (charge, present, ppf_non_vide, flags)
    };

    let t0 = std::time::Instant::now();
    let reseau = match client.resolve_batch(&[canonique.clone()]).await {
        Ok((items, _)) => {
            let latence_ms = t0.elapsed().as_millis() as u64;
            match items.first() {
                Some(item) => {
                    let r = crate::resolver::to_resolution(item, &canonique, now.timestamp());
                    crate::unitaire::Reseau::Repond {
                        champs: crate::unitaire::champs_reseau(&r, now),
                        latence_ms,
                    }
                }
                None => crate::unitaire::Reseau::Echec {
                    message: "L'API n'a rien renvoyé pour cet adressage.".into(),
                },
            }
        }
        Err(e) => crate::unitaire::Reseau::Echec { message: e.to_string() },
    };

    Ok(crate::unitaire::ResolutionUnitaire {
        saisi: saisi.to_string(),
        canonique,
        mode: mode.to_string(),
        reseau,
        annuaire_peppol: crate::unitaire::etat_annuaire_peppol(
            valeur_0225.as_deref(),
            charge,
            present,
        ),
        ppf: crate::unitaire::etat_ppf(valeur_0225.as_deref(), ppf_non_vide, flags.as_ref()),
    })
}

/// Résolution unitaire depuis la loupe de l'en-tête. Consultation seule.
#[tauri::command]
pub async fn resoudre_adressage(
    saisi: String,
    state: State<'_, AppState>,
) -> Result<crate::unitaire::ResolutionUnitaire, String> {
    let cfg = state.current_config()?;
    let mode = if cfg.api.mode == ApiMode::Direct { "direct" } else { "api" };
    let client = state.client()?;
    resoudre_adressage_impl(
        &client,
        &state.store,
        &cfg.ppf.motifs(),
        mode,
        &saisi,
        chrono::Utc::now(),
    )
    .await
}
```

`PpfFlags` dérive déjà `Clone, Copy` (`store.rs:55`) : `.cloned()` compile sans
rien modifier dans `store.rs`.

- [ ] **Step 5 : lancer les tests pour les voir passer**

Run : `cd client/src-tauri && cargo test resoudre_adressage`
Expected : PASS — 3 tests.

- [ ] **Step 6 : enregistrer la commande**

Dans `client/src-tauri/src/lib.rs`, dans `tauri::generate_handler![…]`, ajouter après `commands::test_api,` :

```rust
            commands::resoudre_adressage,
```

- [ ] **Step 7 : vérifier l'ensemble**

Run : `cd client/src-tauri && cargo test`
Expected : PASS — 643 tests, 0 échec.

- [ ] **Step 8 : commit**

```bash
git add client/src-tauri/src/unitaire.rs client/src-tauri/src/commands.rs client/src-tauri/src/lib.rs
git commit -m "feat(superpopaul): commande de résolution unitaire, sans écriture en base"
```

---

### Task 5 : la loupe et la modale

**Files:**
- Modify: `client/src/index.html:17`
- Modify: `client/src/app.js`
- Create: `client/tests/resolution_unitaire.test.js`

- [ ] **Step 1 : ajouter le bouton**

Dans `client/src/index.html`, dans `<span class="cfg-btns">`, AVANT `#btn-settings` :

```html
      <button id="btn-resolve" class="btn-ghost" title="Résoudre un adressage — consultation seule.">🔍</button>
```

Aucune classe de couleur : `.btn-ghost` suffit, la loupe est monochrome comme ⚙.

- [ ] **Step 2 : écrire le test qui échoue**

Créer `client/tests/resolution_unitaire.test.js` :

```js
// Câblage de la loupe : ce que la modale demande au backend, et quand.
//
// Le piège visé : un champ vide qui part quand même sur le réseau, et un échec
// réseau qui masquerait les annuaires locaux — ils répondent sans réseau, les
// cacher priverait l'utilisateur d'informations que la machine possède.

const test = require("node:test");
const assert = require("node:assert/strict");
const { chargerApp } = require("./dom_shim");

const REPONSE = {
  saisi: "552100554",
  canonique: "iso6523-actorid-upis::0225:552100554",
  mode: "api",
  reseau: { etat: "echec", message: "La requête n'a pas atteint l'API (HTTP 403)." },
  annuaire_peppol: { etat: "repond", in_directory: true },
  ppf: { etat: "muette", raison: "annuaire_vide" },
};

function ouvrir(reponse = REPONSE) {
  const ctx = chargerApp();
  const appels = [];
  ctx.repondreAux((cmd, args) => {
    appels.push([cmd, args]);
    return cmd === "resoudre_adressage" ? ctx.evaluer(`(${JSON.stringify(reponse)})`) : null;
  });
  ctx.$("btn-resolve").click();
  return { ctx, appels };
}

test("une saisie vide ne part pas sur le réseau", async () => {
  const { ctx, appels } = ouvrir();
  await ctx.$("resolve-go").click();
  assert.equal(
    appels.filter(([c]) => c === "resoudre_adressage").length,
    0,
    "aucun appel ne doit partir sans adressage",
  );
});

test("l'adressage saisi est transmis tel quel au backend", async () => {
  const { ctx, appels } = ouvrir();
  ctx.$("resolve-input").value = "552100554";
  await ctx.$("resolve-go").click();
  const appel = appels.find(([c]) => c === "resoudre_adressage");
  assert.ok(appel, "la commande doit être appelée");
  assert.equal(appel[1].saisi, "552100554");
});

test("un échec réseau n'empêche pas d'afficher les annuaires", async () => {
  const { ctx } = ouvrir();
  ctx.$("resolve-input").value = "552100554";
  await ctx.$("resolve-go").click();
  const txt = ctx.$("resolve-result").textContent;
  assert.match(txt, /HTTP 403/, `le message d'échec doit être lisible : ${txt}`);
  assert.match(txt, /in_directory/, `l'annuaire Peppol doit rester affiché : ${txt}`);
});
```

- [ ] **Step 3 : lancer le test pour le voir échouer**

Run : `cd client && node --test "tests/resolution_unitaire.test.js"`
Expected : FAIL — `Cannot read properties of null` sur `$("btn-resolve")` (le bouton existe en HTML mais aucun écouteur ne construit la modale).

- [ ] **Step 4 : écrire l'implémentation**

Dans `client/src/app.js`, à la suite du bloc « Réglages : test API et calibrage » :

```js
// --- Loupe : résolution d'un adressage unitaire ---------------------------
// Consultation seule : la commande n'écrit rien, et l'écran le rappelle.
$("btn-resolve").addEventListener("click", () => {
  const champ = h("input", {
    type: "text", id: "resolve-input",
    placeholder: "SIREN, 0225:… ou identifiant complet",
  });
  const sortie = h("div", { id: "resolve-result" });
  const go = h("button", {
    id: "resolve-go", class: "primary",
    onclick: () => lancerResolution(champ, sortie, go),
  }, "Résoudre");
  champ.addEventListener("keydown", (ev) => {
    if (ev.key === "Enter") lancerResolution(champ, sortie, go);
  });
  modal(
    h("div", { class: "modal-h" },
      h("h3", {}, "Résoudre un adressage"),
      h("span", { class: "tag gold" }, "consultation seule"),
      h("button", { class: "btn-ghost", onclick: closeModal }, "✕")),
    h("div", { class: "resolve-saisie" }, champ, go),
    sortie,
    h("p", { class: "resolve-hint" },
      "Le résultat n'est pas enregistré : consulter un compte ici ne le retire "
      + "pas du périmètre d'un run futur."));
  champ.focus();
});

async function lancerResolution(champ, sortie, go) {
  const saisi = champ.value.trim();
  if (!saisi) return;
  go.disabled = true;
  sortie.replaceChildren(h("div", { class: "resolve-spin" }, "Résolution en cours…"));
  try {
    const r = await invoke("resoudre_adressage", { saisi });
    sortie.replaceChildren(...rendreResolution(r));
  } catch (err) {
    // Erreur de commande (saisie refusée, config absente) : le backend rend
    // déjà un texte en français, on ne le réécrit pas.
    sortie.replaceChildren(h("div", { class: "banner err" }, `❌ ${err}`));
  } finally {
    go.disabled = false;
  }
}
```

- [ ] **Step 5 : lancer le test — les deux premiers passent, le troisième échoue**

Run : `cd client && node --test "tests/resolution_unitaire.test.js"`
Expected : 2 PASS, 1 FAIL — `rendreResolution is not defined`. C'est la Task 6.

- [ ] **Step 6 : commit**

```bash
git add client/src/index.html client/src/app.js client/tests/resolution_unitaire.test.js
git commit -m "wip(superpopaul): loupe et modale de résolution unitaire"
```

---

### Task 6 : le rendu du résultat

**Files:**
- Modify: `client/src/app.js`
- Modify: `client/src/styles.css`

- [ ] **Step 1 : écrire le rendu**

Dans `client/src/app.js`, à la suite de `lancerResolution` :

```js
/** Définitions des champs, RECOPIÉES de `docs/legende_champs.md` — une seule
 *  source pour le CSV, le PDF de légende et cet écran. Toute modification là-bas
 *  doit être reportée ici : `client/tests/legende_parite.test.js` échoue sinon. */
const LEGENDE = {
  in_peppol: "existe — L'adressage est-il provisionné dans le réseau Peppol (le SMP répond pour cet identifiant).",
  pa_code: "code PA — Code du point d'accès (Access Point) qui dessert l'adressage.",
  pa_name: "nom PA — Nom du point d'accès.",
  pa_country: "pays PA — Code pays du point d'accès.",
  ubl_extended: "CTC-FR — L'adressage déclare-t-il le support de l'extension française France Invoice UBL Extension (CTC-FR).",
  ctc_activation: "activation CTC — Date d'activation déclarée du support CTC (chaîne SMP brute, ISO 8601).",
  ctc_expiration: "expiration CTC — Date d'expiration déclarée du support.",
  ctc_status: "état CTC — État du support calculé à l'instant de l'export à partir des dates ci-dessus.",
  in_directory: "annuaire Peppol — L'adressage 0225 figure-t-il dans l'annuaire Peppol chargé.",
  annuaire_ppf: "annuaire PPF — Adressage présent dans l'annuaire PPF chargé (au moins une ligne).",
  ppf_active: "PPF actif — Au moins une ligne à un motif de présence actif (ensemble configurable dans les réglages, par défaut C / P).",
  pdp_definie: "PDP définie — Au moins une ligne avec une PDP réelle (pdp_fictive = 0).",
  ppf_usable: "PPF utilisable — Au moins une même ligne à un motif actif configuré (défaut C / P) ET PDP réelle (pdp_fictive = 0).",
};

/** Libellé humain des deux champs de vedette (2e ligne, sous le nom technique). */
const VEDETTE = { ctc_status: "état CTC", ppf_usable: "PPF utilisable" };

/** Pourquoi une source se tait. Jamais « false » : « je ne sais pas » et « non »
 *  sont deux réponses différentes. */
const MUETTE = {
  annuaire_non_charge: "annuaire jamais chargé",
  annuaire_vide: "annuaire vide",
  hors_perimetre_0225: "hors périmètre des annuaires (0225)",
};

/** Classe de couleur d'un verdict. Quatre issues, pas deux : `later` basculera
 *  seul le jour de l'activation, le peindre en rouge dirait « disqualifié ». */
function classeVerdict(nom, valeur) {
  if (valeur === null || valeur === undefined || valeur === "") return "verdict-nul";
  if (nom === "ctc_status") {
    return { ready: "verdict-ok", later: "verdict-later", expired: "verdict-ko" }[valeur]
      || "verdict-nul";
  }
  return valeur === true ? "verdict-ok" : "verdict-ko";
}

/** Une ligne de champ. `vedette` agrandit et colore ; sinon rendu discret. */
function ligneChamp(nom, valeur, vedette = false) {
  const texte = valeur === null || valeur === undefined || valeur === "" ? "—" : String(valeur);
  const cle = vedette
    ? h("td", { class: "k" }, nom, h("span", { class: "lib" }, VEDETTE[nom]))
    : h("td", { class: "k" }, nom);
  const val = vedette
    ? h("td", { class: "v" }, h("span", { class: classeVerdict(nom, valeur) }, texte))
    : h("td", { class: `v ${valeur === true ? "t" : valeur === false ? "f" : ""}` }, texte);
  return h("tr", { title: LEGENDE[nom], class: vedette ? "cle" : "" }, cle, val);
}

/** Une source muette : la raison, jamais une valeur. */
function ligneMuette(nom, raison, vedette = false) {
  const cle = vedette
    ? h("td", { class: "k" }, nom, h("span", { class: "lib" }, VEDETTE[nom]))
    : h("td", { class: "k" }, nom);
  return h("tr", { title: LEGENDE[nom], class: vedette ? "cle" : "" },
    cle,
    h("td", { class: "v" }, h("span", { class: "verdict-nul" }, MUETTE[raison] || raison)));
}

function section(titre, ...lignes) {
  return h("div", { class: "resolve-sect" },
    h("div", { class: "resolve-sect-h" }, titre),
    h("table", {}, ...lignes));
}

/** Rendu complet. Les trois sections sont toujours présentes : une source
 *  muette se dit, elle ne disparaît pas. */
function rendreResolution(r) {
  const out = [];
  out.push(h("p", { class: "resolve-canon" },
    "Résolu comme ", h("span", { class: "v" }, r.canonique), ` · mode ${r.mode}`));

  if (r.reseau.etat === "repond") {
    const c = r.reseau.champs;
    out.push(section(`Réseau Peppol · ${c.note ? c.note : `${r.reseau.latence_ms} ms`}`,
      ligneChamp("in_peppol", c.in_peppol),
      ligneChamp("pa_code", c.pa_code),
      ligneChamp("pa_name", c.pa_name),
      ligneChamp("pa_country", c.pa_country),
      ligneChamp("ubl_extended", c.ubl_extended),
      ligneChamp("ctc_activation", c.ctc_activation),
      ligneChamp("ctc_expiration", c.ctc_expiration),
      ligneChamp("ctc_status", c.ctc_status, true)));
  } else {
    out.push(h("div", { class: "banner err" }, `❌ ${r.reseau.message}`));
  }

  out.push(r.annuaire_peppol.etat === "repond"
    ? section("Annuaire Peppol", ligneChamp("in_directory", r.annuaire_peppol.in_directory))
    : section("Annuaire Peppol", ligneMuette("in_directory", r.annuaire_peppol.raison)));

  out.push(r.ppf.etat === "repond"
    ? section("Annuaire PPF",
      ligneChamp("annuaire_ppf", r.ppf.annuaire_ppf),
      ligneChamp("ppf_active", r.ppf.ppf_active),
      ligneChamp("pdp_definie", r.ppf.pdp_definie),
      ligneChamp("ppf_usable", r.ppf.ppf_usable, true))
    : section("Annuaire PPF",
      ligneMuette("annuaire_ppf", r.ppf.raison),
      ligneMuette("ppf_usable", r.ppf.raison, true)));

  return out;
}
```

- [ ] **Step 2 : lancer les tests pour les voir passer**

Run : `cd client && node --test "tests/resolution_unitaire.test.js"`
Expected : PASS — 3 tests.

- [ ] **Step 3 : ajouter les styles**

Dans `client/src/styles.css`, à la fin :

```css
/* --- Loupe : résolution d'un adressage unitaire ------------------------- */
.resolve-saisie{display:flex;gap:8px;margin-bottom:6px}
#resolve-input{flex:1}
.resolve-canon{font-size:12px;color:var(--muted);margin:0 0 14px}
.resolve-canon .v{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;color:var(--pid)}
.resolve-hint{font-size:12px;color:var(--muted);margin:14px 0 0;padding-top:11px;
              border-top:1px solid var(--border)}
.resolve-spin{color:var(--muted);font-style:italic;padding:20px 0;text-align:center}
.resolve-sect{border:1px solid var(--border);border-radius:8px;margin-bottom:9px;overflow:hidden}
.resolve-sect-h{padding:7px 12px;background:rgba(255,255,255,.025);
                border-bottom:1px solid var(--border);font-size:12px;text-transform:uppercase;
                letter-spacing:.07em;color:var(--muted)}
.resolve-sect table{width:100%;border-collapse:collapse;font-size:12.5px}
.resolve-sect td{padding:5px 12px;border-bottom:1px solid rgba(43,55,82,.55)}
.resolve-sect tr:last-child td{border-bottom:0}
/* La ligne entière est survolable : viser un nom de huit caractères est pénible. */
.resolve-sect tr[title]{cursor:help}
.resolve-sect tr[title]:hover td{background:rgba(255,255,255,.035)}
.resolve-sect td.k{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;
                   color:var(--muted);width:170px}
.resolve-sect td.v{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;color:var(--fg)}
.resolve-sect td.v.t{color:var(--green)}
.resolve-sect td.v.f{color:var(--muted)}
/* Les deux champs qui décident de l'éligibilité. */
.resolve-sect tr.cle td{background:rgba(217,168,63,.07);
                        border-top:1px solid rgba(217,168,63,.25);
                        border-bottom:1px solid rgba(217,168,63,.25);padding:9px 12px}
.resolve-sect tr.cle td.k{color:var(--fg);font-weight:600}
.resolve-sect tr.cle td.k .lib{display:block;font-family:inherit;font-size:11px;
                               font-weight:400;color:var(--muted);text-transform:uppercase;
                               letter-spacing:.06em}
.resolve-sect tr.cle td.v{font-size:15px;font-weight:700}
.verdict-ok{color:var(--green)}
.verdict-ko{color:var(--red)}
.verdict-later{color:var(--green-later)}
.verdict-nul{color:var(--amber);font-style:italic;font-size:13px;font-weight:400;
             font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif}
```

Les quatre variables employées existent déjà dans `:root` de `styles.css` —
`--amber` et `--red` (l. 14), `--pid` (l. 17), `--green-later` (l. 22) : rien à
déclarer.

- [ ] **Step 4 : lancer toute la suite JS**

Run : `cd client && node --test "tests/*.test.js"`
Expected : PASS — 93 tests (90 + 3), 0 échec.

- [ ] **Step 5 : commit**

```bash
git add client/src/app.js client/src/styles.css
git commit -m "feat(superpopaul): rendu de la résolution unitaire, verdicts colorés et infobulles"
```

---

### Task 7 : les infobulles ne peuvent pas diverger de la légende

**Files:**
- Create: `client/tests/legende_parite.test.js`

- [ ] **Step 1 : écrire le test**

Le projet a déjà payé une divergence de libellé entre le JS et le Rust
(`active_label` ↔ `ppfActiveTag`). `LEGENDE` recopie `docs/legende_champs.md` :
sans garde, les deux dériveront.

Créer `client/tests/legende_parite.test.js` :

```js
// Les infobulles de la loupe recopient docs/legende_champs.md. Ce test est le
// seul lien mécanique entre les deux : sans lui, corriger la légende laisserait
// l'application afficher l'ancienne définition, indéfiniment.

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { chargerApp } = require("./dom_shim");

const MD = fs.readFileSync(
  path.join(__dirname, "..", "..", "docs", "legende_champs.md"), "utf8");

/** Description du champ dans le tableau markdown : | `nom` | libellé | description | … */
function ligneLegende(nom) {
  const re = new RegExp(`^\\|\\s*\`${nom}\`\\s*\\|([^|]*)\\|([^|]*)\\|`, "m");
  const m = MD.match(re);
  return m && { libelle: m[1].trim(), description: m[2].trim() };
}

test("chaque infobulle reprend le libellé et la description de la légende", () => {
  const ctx = chargerApp();
  const legende = ctx.evaluer("LEGENDE");
  const noms = Object.keys(legende);
  assert.ok(noms.length >= 13, `13 champs attendus, ${noms.length} trouvés`);

  for (const nom of noms) {
    const source = ligneLegende(nom);
    assert.ok(source, `${nom} est absent de docs/legende_champs.md`);
    // Le markdown porte des **gras** et des `codes` que l'infobulle aplatit :
    // on compare le texte nu.
    const nu = (s) => s.replace(/[*`]/g, "").replace(/\s+/g, " ").trim();
    assert.equal(
      nu(legende[nom]),
      nu(`${source.libelle} — ${source.description}`),
      `l'infobulle de ${nom} a divergé de docs/legende_champs.md`,
    );
  }
});
```

- [ ] **Step 2 : lancer le test**

Run : `cd client && node --test "tests/legende_parite.test.js"`
Expected : le test passe, OU échoue en nommant le champ dont le texte diffère.
Dans ce second cas, corriger `LEGENDE` dans `app.js` pour coller au markdown —
**jamais l'inverse** : le markdown est la source, il alimente aussi le PDF.

- [ ] **Step 3 : prouver que le test peut échouer**

Modifier temporairement une entrée de `LEGENDE` dans `app.js` (par exemple
`pa_name: "nom PA — Autre chose."`), relancer :

Run : `cd client && node --test "tests/legende_parite.test.js"`
Expected : FAIL — « l'infobulle de pa_name a divergé de docs/legende_champs.md ».
Puis restaurer le texte exact et relancer : PASS.

- [ ] **Step 4 : commit**

```bash
git add client/tests/legende_parite.test.js
git commit -m "test(superpopaul): les infobulles de la loupe ne peuvent plus diverger de la légende"
```

---

### Task 8 : vérification d'ensemble

- [ ] **Step 1 : suite Rust complète**

Run : `cd client/src-tauri && cargo test`
Expected : `test result: ok. 643 passed; 0 failed`

- [ ] **Step 2 : suite JS complète**

Run : `cd client && node --test "tests/*.test.js"`
Expected : `pass 94`, `fail 0`

- [ ] **Step 3 : parcours réel**

Lancer l'application, ouvrir la loupe et vérifier, dans cet ordre :
1. un SIREN nu → la forme canonique affichée porte `::0225:` ;
2. le survol d'une ligne montre sa définition ;
3. `ctc_status` et `ppf_usable` sont agrandis et colorés ;
4. sans annuaire PPF chargé, la section PPF dit « annuaire vide » et **pas** `false` ;
5. un adressage `0007:…` dit « hors périmètre des annuaires (0225) » ;
6. après la consultation, un run couvre toujours ce compte (rien n'a été écrit).

Le point 6 est le seul que les tests ne prouvent pas : ils vérifient qu'aucune
écriture n'est demandée, pas qu'un run ultérieur se comporte comme avant.
