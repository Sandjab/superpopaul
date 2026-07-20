# Motifs PPF actifs configurables — Plan d'implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rendre configurable l'ensemble des motifs de présence PPF comptés comme « actifs » (aujourd'hui `C`/`P` en dur), via un réglage global `Settings.ppf.active_motifs` (défaut `CP`), alimentant `ppf_active` et, par héritage, `ppf_usable`.

**Architecture:** Nouvelle struct `PpfConfig { active_motifs: String }` partagée entre `Config` (runtime) et `Settings` (persisté YAML), sur le modèle d'`ApiConfig`. Le calcul `store::ppf_flags` prend l'ensemble des motifs en paramètre ; le SQL `motif IN ('C','P')` en dur devient une clause paramétrée `UPPER(motif) IN (…)`. Aucune migration : les drapeaux sont des agrégats SQL calculés à la volée.

**Tech Stack:** Rust (rusqlite, serde, serde_yaml), frontend vanilla JS, weasyprint pour le PDF de doc.

**Spec de référence:** `docs/superpowers/specs/2026-07-20-motifs-ppf-actifs-configurables-design.md`

---

## Structure des fichiers

| Fichier | Rôle dans ce chantier |
|---|---|
| `client/src-tauri/src/config.rs` | Struct `PpfConfig`, défaut, `motifs()`, `validate_ppf`, champ `ppf` dans `Config` et `Settings`, appels de validation |
| `client/src-tauri/src/store.rs` | `ppf_flags` paramétré par les motifs, SQL dynamique |
| `client/src-tauri/src/commands.rs` | Passer les motifs (`cfg.ppf.motifs()`) aux 3 sites d'appel ; param sur `coverage_from_scan` / `securisation_from_scan` |
| `client/src/index.html` | Champ « Motifs PPF actifs » dans le formulaire de réglages (après maquette) |
| `client/src/app.js` | État par défaut `ppf`, câblage formulaire ↔ Settings |
| `client/src/columns.js` | Tooltips PPF génériques |
| `docs/legende_champs.md` + `docs/legende_champs.pdf` | Doc mise à jour + PDF régénéré |

---

## Task 1: `PpfConfig` — struct, défaut, normalisation, validation

**Files:**
- Modify: `client/src-tauri/src/config.rs` (ajouter après le bloc `PeppolField`, ~ligne 255)
- Test: `client/src-tauri/src/config.rs` (module `#[cfg(test)]` existant en bas)

- [ ] **Step 1: Écrire les tests qui échouent**

Ajouter dans le module de tests de `config.rs` :

```rust
#[test]
fn ppf_motifs_normalise_majuscule_espaces_dedup() {
    let p = PpfConfig { active_motifs: "cp P ".into() };
    assert_eq!(p.motifs(), vec!["C".to_string(), "P".to_string()]);
}

#[test]
fn ppf_config_defaut_est_cp() {
    assert_eq!(PpfConfig::default().active_motifs, "CP");
    assert_eq!(PpfConfig::default().motifs(), vec!["C".to_string(), "P".to_string()]);
}

#[test]
fn validate_ppf_refuse_vide_et_non_lettre_accepte_cpn() {
    assert!(validate_ppf(&PpfConfig { active_motifs: "".into() }).is_err());
    assert!(validate_ppf(&PpfConfig { active_motifs: "   ".into() }).is_err());
    assert!(validate_ppf(&PpfConfig { active_motifs: "C1".into() }).is_err());
    assert!(validate_ppf(&PpfConfig { active_motifs: "CPN".into() }).is_ok());
}
```

- [ ] **Step 2: Lancer les tests → échec de compilation (symboles absents)**

Run: `cd client/src-tauri && cargo test config:: 2>&1 | tail -15`
Expected: FAIL — `cannot find type PpfConfig` / `cannot find function validate_ppf`.

- [ ] **Step 3: Implémenter `PpfConfig` + `validate_ppf`**

Ajouter dans `config.rs` (hors module de tests, après l'enum `PeppolField`) :

```rust
fn ppf_active_motifs_default() -> String {
    "CP".to_string()
}

/// Règle d'interprétation de l'annuaire PPF : quels motifs de présence
/// comptent comme « actifs » (alimente `ppf_active` et, par héritage,
/// `ppf_usable`). Réglage global, persisté ; défaut historique `CP`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PpfConfig {
    #[serde(default = "ppf_active_motifs_default")]
    pub active_motifs: String,
}

impl Default for PpfConfig {
    fn default() -> Self {
        PpfConfig { active_motifs: ppf_active_motifs_default() }
    }
}

impl PpfConfig {
    /// Ensemble des motifs actifs normalisés : majuscules, espaces retirés,
    /// dédupliqués. Précondition d'usage : `validate_ppf` garantit un ensemble
    /// non vide de lettres avant tout calcul.
    pub fn motifs(&self) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for ch in self.active_motifs.chars() {
            if ch.is_whitespace() {
                continue;
            }
            let m = ch.to_ascii_uppercase().to_string();
            if seen.insert(m.clone()) {
                out.push(m);
            }
        }
        out
    }

    fn is_default(&self) -> bool {
        *self == PpfConfig::default()
    }
}

fn validate_ppf(ppf: &PpfConfig) -> Result<(), String> {
    let motifs = ppf.motifs();
    if motifs.is_empty() {
        return Err("motifs PPF actifs : au moins une lettre (ex. CP)".into());
    }
    for m in &motifs {
        if !m.chars().all(|c| c.is_ascii_alphabetic()) {
            return Err(format!(
                "motifs PPF actifs : caractère invalide « {m} » (lettres A-Z uniquement)"
            ));
        }
    }
    Ok(())
}
```

- [ ] **Step 4: Lancer les tests → succès**

Run: `cd client/src-tauri && cargo test config::tests::ppf 2>&1 | tail -10 && cargo test config::tests::validate_ppf 2>&1 | tail -10`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add client/src-tauri/src/config.rs
git commit -m "feat(superpopaul): PpfConfig — motifs actifs (normalisation + validation, TDD)"
```

---

## Task 2: Intégrer `ppf` dans `Config` et `Settings` (rétro-compat)

**Files:**
- Modify: `client/src-tauri/src/config.rs` — struct `Config` (~L6-11), `Config::validate` (~L283), struct `Settings` (~L300-304), `Settings::validate` (~L321)
- Test: `client/src-tauri/src/config.rs` (module de tests)

- [ ] **Step 1: Écrire les tests qui échouent**

```rust
#[test]
fn settings_sans_ppf_prend_cp_et_ne_reecrit_pas_le_defaut() {
    let yaml = "version: 1\n\
                api:\n  url: \"x\"\n  key: \"\"\n  batch_size: 50\n  concurrency: 8\n  refresh_days: 30\n\
                output:\n  timestamp_suffix: false\n";
    let s: Settings = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(s.ppf.active_motifs, "CP", "YAML sans ppf → défaut CP");
    let out = serde_yaml::to_string(&s).unwrap();
    assert!(!out.contains("ppf"), "ppf par défaut ne doit pas être sérialisé");
}

#[test]
fn settings_ppf_custom_round_trip_et_valide() {
    let yaml = "version: 1\n\
                api:\n  url: \"x\"\n  key: \"\"\n  batch_size: 50\n  concurrency: 8\n  refresh_days: 30\n\
                output:\n  timestamp_suffix: false\n\
                ppf:\n  active_motifs: \"CPN\"\n";
    let s: Settings = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(s.ppf.active_motifs, "CPN");
    s.validate().unwrap();
    assert!(serde_yaml::to_string(&s).unwrap().contains("CPN"));
}

#[test]
fn settings_validate_refuse_ppf_vide() {
    let yaml = "version: 1\n\
                api:\n  url: \"x\"\n  key: \"\"\n  batch_size: 50\n  concurrency: 8\n  refresh_days: 30\n\
                output:\n  timestamp_suffix: false\n\
                ppf:\n  active_motifs: \"\"\n";
    let s: Settings = serde_yaml::from_str(yaml).unwrap();
    assert!(s.validate().is_err(), "ppf vide doit être rejeté à la validation");
}
```

- [ ] **Step 2: Lancer → échec**

Run: `cd client/src-tauri && cargo test config::tests::settings_ 2>&1 | tail -15`
Expected: FAIL — `no field ppf on type Settings`.

- [ ] **Step 3: Ajouter le champ `ppf` et les appels de validation**

Dans `struct Config` (après `pub output: OutputConfig,`) :

```rust
    #[serde(default, skip_serializing_if = "PpfConfig::is_default")]
    pub ppf: PpfConfig,
```

Dans `Config::validate`, remplacer les deux dernières lignes du corps :

```rust
        validate_api(&self.api)?;
        validate_suffix(&self.output.suffix)
```
par :
```rust
        validate_api(&self.api)?;
        validate_suffix(&self.output.suffix)?;
        validate_ppf(&self.ppf)
```

Dans `struct Settings` (après `pub output: OutputSettings,`) :

```rust
    #[serde(default, skip_serializing_if = "PpfConfig::is_default")]
    pub ppf: PpfConfig,
```

Dans `Settings::validate` — remplacer le corps par :

```rust
    pub fn validate(&self) -> Result<(), String> {
        validate_api(&self.api)?;
        validate_suffix(&self.output.suffix)?;
        validate_ppf(&self.ppf)
    }
```

- [ ] **Step 4: Lancer → succès (et suite config complète)**

Run: `cd client/src-tauri && cargo test config:: 2>&1 | tail -12`
Expected: PASS (dont les 3 nouveaux + les tests config existants).

- [ ] **Step 5: Commit**

```bash
git add client/src-tauri/src/config.rs
git commit -m "feat(superpopaul): Config/Settings.ppf — réglage global rétro-compatible (défaut CP)"
```

---

## Task 3: `ppf_flags` paramétré + câblage des 3 appelants

> Changer la signature de `ppf_flags` casse la compilation des appelants : cette tâche modifie `store.rs` **et** `commands.rs` dans le même commit pour que `cargo test` compile.

**Files:**
- Modify: `client/src-tauri/src/store.rs` — `ppf_flags` (~L428-460) ; tests existants `ppf_flags_calcule_les_quatre_drapeaux` (~L895) et `ppf_flags_traverse_plusieurs_lots` (~L927)
- Modify: `client/src-tauri/src/commands.rs` — `coverage_from_scan` (L100, appel L121), `securisation_from_scan` (L138, appel L153), export (appel L677) ; appelants L297, L579-581
- Test: `client/src-tauri/src/store.rs` (module de tests)

- [ ] **Step 1: Écrire le test qui échoue (motif configurable)**

Ajouter dans le module de tests de `store.rs` :

```rust
#[test]
fn ppf_flags_honore_les_motifs_configures() {
    let s = Store::open_in_memory().unwrap();
    // id_n : une seule ligne au motif N, PDP réelle (pdp_fictive = 0).
    s.ingest_ppf("f.csv", "h", &[ppf_row("id_n", "N", 0)], 1, 1).unwrap();
    let ids = vec!["id_n".to_string()];

    // Défaut CP : N n'est pas actif (ni usable), mais pdp_definie reste vrai.
    let cp = s.ppf_flags(&ids, &["C".to_string(), "P".to_string()]).unwrap();
    assert_eq!(
        cp.get("id_n").copied(),
        Some(PpfFlags { in_ppf: true, active: false, pdp_definie: true, usable: false })
    );

    // CPN : N devient actif ET usable (pdp réelle sur la même ligne).
    let cpn = s
        .ppf_flags(&ids, &["C".to_string(), "P".to_string(), "N".to_string()])
        .unwrap();
    assert_eq!(
        cpn.get("id_n").copied(),
        Some(PpfFlags { in_ppf: true, active: true, pdp_definie: true, usable: true })
    );
}

#[test]
fn ppf_flags_insensible_a_la_casse_du_motif() {
    let s = Store::open_in_memory().unwrap();
    s.ingest_ppf("f.csv", "h", &[ppf_row("id_low", "c", 0)], 1, 1).unwrap();
    // Annuaire en minuscule « c », réglage « C » → actif (UPPER des deux côtés).
    let m = s.ppf_flags(&["id_low".to_string()], &["C".to_string()]).unwrap();
    assert!(m.get("id_low").unwrap().active);
}
```

- [ ] **Step 2: Lancer → échec (arité)**

Run: `cd client/src-tauri && cargo test store::tests::ppf_flags_honore 2>&1 | tail -15`
Expected: FAIL — `this function takes 1 argument but 2 arguments were supplied`.

- [ ] **Step 3: Paramétrer `ppf_flags` (store.rs)**

Remplacer le corps de `ppf_flags` par :

```rust
    /// Drapeaux PPF pour chaque `identifiant` présent en table. `active_motifs`
    /// (déjà normalisés en majuscules, non vides — cf. PpfConfig/validate_ppf)
    /// définit les motifs « actifs » qui alimentent `active` et `usable`.
    /// `ppf_usable` exige (motif actif) ET pdp_fictive=0 sur la MÊME ligne.
    pub fn ppf_flags(
        &self,
        identifiants: &[String],
        active_motifs: &[String],
    ) -> Result<HashMap<String, PpfFlags>, String> {
        let mut out = HashMap::new();
        if active_motifs.is_empty() {
            return Err("ppf_flags : aucun motif actif (précondition violée)".into());
        }
        let motif_ph = vec!["?"; active_motifs.len()].join(",");
        for chunk in identifiants.chunks(500) {
            let id_ph = vec!["?"; chunk.len()].join(",");
            let sql = format!(
                "SELECT identifiant, \
                        MAX(UPPER(motif) IN ({motif_ph})), \
                        MAX(pdp_fictive = 0), \
                        MAX(UPPER(motif) IN ({motif_ph}) AND pdp_fictive = 0) \
                 FROM ppf_directory WHERE identifiant IN ({id_ph}) \
                 GROUP BY identifiant"
            );
            let mut stmt = self.conn.prepare_cached(&sql).map_err(|e| e.to_string())?;
            // Params positionnels : motifs (1re clause), motifs (2e clause), identifiants.
            let mut params: Vec<&str> = Vec::with_capacity(active_motifs.len() * 2 + chunk.len());
            params.extend(active_motifs.iter().map(String::as_str));
            params.extend(active_motifs.iter().map(String::as_str));
            params.extend(chunk.iter().map(String::as_str));
            let rows = stmt
                .query_map(rusqlite::params_from_iter(params), |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        PpfFlags {
                            in_ppf: true,
                            active: r.get::<_, i64>(1)? != 0,
                            pdp_definie: r.get::<_, i64>(2)? != 0,
                            usable: r.get::<_, i64>(3)? != 0,
                        },
                    ))
                })
                .map_err(|e| e.to_string())?;
            for row in rows {
                let (id, flags) = row.map_err(|e| e.to_string())?;
                out.insert(id, flags);
            }
        }
        Ok(out)
    }
```

- [ ] **Step 4: Mettre à jour les 2 tests existants de `ppf_flags`**

Dans `ppf_flags_calcule_les_quatre_drapeaux`, remplacer :

```rust
        let m = s.ppf_flags(&ids).unwrap();
```
par :
```rust
        let m = s.ppf_flags(&ids, &["C".to_string(), "P".to_string()]).unwrap();
```

Dans `ppf_flags_traverse_plusieurs_lots`, remplacer :

```rust
        let m = s.ppf_flags(&ids).unwrap();
```
par :
```rust
        let m = s.ppf_flags(&ids, &["C".to_string(), "P".to_string()]).unwrap();
```

- [ ] **Step 5: Câbler les 3 appelants (commands.rs)**

Dans `coverage_from_scan` (signature L100), ajouter un paramètre :

```rust
fn coverage_from_scan(
    store: &Store,
    pids: &[String],
    line_counts: &HashMap<String, u64>,
    active_motifs: &[String],
) -> Result<crate::coverage::Coverage, String> {
```
et à l'appel interne (L121) :
```rust
        Some(store.ppf_flags(&values, active_motifs)?)
```

Dans `securisation_from_scan` (signature L138), ajouter le paramètre :

```rust
fn securisation_from_scan(
    store: &Store,
    pids: &[String],
    line_counts: &HashMap<String, u64>,
    now: chrono::DateTime<chrono::Utc>,
    active_motifs: &[String],
) -> Result<Option<crate::securisation::Securisation>, String> {
```
et à l'appel interne (L153) :
```rust
    let ppf = store.ppf_flags(&values, active_motifs)?;
```

Dans `analyze_input` (appel L297, `cfg` en scope L288) :
```rust
        let coverage = coverage_from_scan(&store_g, &pids, &line_counts, &cfg.ppf.motifs())?;
```

Dans `export_report` (appels L579-581, `cfg` en scope L568) :
```rust
                let cov = coverage_from_scan(&store_g, &pids, &line_counts, &cfg.ppf.motifs())
```
```rust
                let secu = securisation_from_scan(&store_g, &pids, &line_counts, now_utc, &cfg.ppf.motifs())
```

Dans l'export (`generate_output`, appel L677, `cfg` en scope L625) :
```rust
                Some(s.ppf_flags(&ids, &cfg.ppf.motifs())?)
```

- [ ] **Step 6: Lancer toute la suite → succès**

Run: `cd client/src-tauri && cargo test 2>&1 | tail -8`
Expected: PASS (compile + tous les tests, dont les 2 nouveaux store + les existants mis à jour).

- [ ] **Step 7: Clippy sur le neuf**

Run: `cd client/src-tauri && cargo clippy 2>&1 | tail -15`
Expected: aucun nouveau lint sur `store.rs`/`commands.rs`/`config.rs` (les 5 lints préexistants connus sont hors périmètre).

- [ ] **Step 8: Commit**

```bash
git add client/src-tauri/src/store.rs client/src-tauri/src/commands.rs
git commit -m "feat(superpopaul): ppf_flags paramétré par les motifs actifs + câblage (TDD)"
```

---

## Task 4: UI — maquette puis champ « Motifs PPF actifs »

> Convention projet : **maquette HTML validée avant tout code d'IHM** (mémoire « maquette avant code UI »). Cette tâche a un checkpoint de validation utilisateur.

**Files:**
- Create (temporaire): `scratchpad/maquette-reglages-ppf.html`
- Modify: `client/src/index.html` (formulaire de réglages, près du champ `out-suffix`)
- Modify: `client/src/app.js` — état par défaut (~L37), `syncSettingsForm` (~L389), `fillSettingsForm` (~L419), `currentSettings` (~L515), `applySettings` (~L523)

- [ ] **Step 1: Maquette de l'écran réglages avec le nouveau champ**

Produire une maquette HTML statique reprenant le formulaire de réglages existant (identité « Bleu nuit & or ») avec le nouveau champ **« Motifs PPF actifs »** (input texte court, défaut `CP`, aide « lettres des motifs de présence comptés comme actifs, ex. CP ou CPN »). Placer le champ dans la section « sortie »/« annuaire » du formulaire.

- [ ] **Step 2: CHECKPOINT — faire valider la maquette par l'utilisateur**

Présenter la maquette (go explicite requis) avant d'écrire le code. Ne pas continuer sans validation.

- [ ] **Step 3: Ajouter l'input dans `index.html`**

Dans le formulaire de réglages, après le groupe du suffixe (`out-suffix`), insérer un groupe :

```html
<label for="ppf-motifs">Motifs PPF actifs</label>
<input id="ppf-motifs" type="text" maxlength="16" placeholder="CP"
       title="Lettres des motifs de présence PPF comptés comme actifs (ex. CP, CPN).">
```
(structure exacte alignée sur la maquette validée.)

- [ ] **Step 4: Câbler l'état par défaut et le formulaire (`app.js`)**

État par défaut — dans l'objet `config` (~L36-38), ajouter après `output: {...}` :

```js
    ppf: { active_motifs: "CP" },
```

`syncSettingsForm` (~L389) — ajouter :

```js
  c.ppf.active_motifs = $("ppf-motifs").value.trim();
```

`fillSettingsForm` (~L419) — ajouter :

```js
  $("ppf-motifs").value = c.ppf.active_motifs;
```

`currentSettings` (~L515) — inclure `ppf` dans l'objet envoyé à `save_settings` :

```js
  const { active_motifs } = c.ppf;
  return { version: c.version, api: {...}, output: { dir, suffix, timestamp_suffix },
           ppf: { active_motifs } };
```
(adapter à la forme exacte de l'objet retourné existant — ajouter la clé `ppf`.)

`applySettings` (~L523) — répercuter `ppf` chargé dans l'état :

```js
  if (s.ppf) state.config.ppf.active_motifs = s.ppf.active_motifs;
```
(si les Settings chargés n'ont pas `ppf`, garder le défaut `CP`.)

- [ ] **Step 5: CHECKPOINT — validation GUI en app par l'utilisateur**

L'assistant ne pilote pas la fenêtre Tauri native : l'utilisateur lance l'app, ouvre les réglages, saisit `CPN`, enregistre, rouvre → la valeur persiste ; un export/rapport reflète les nouveaux motifs. Vérifier aussi qu'une valeur vide affiche l'erreur de validation renvoyée par `save_settings`.

- [ ] **Step 6: Commit**

```bash
git add client/src/index.html client/src/app.js
git commit -m "feat(superpopaul): réglage « Motifs PPF actifs » dans l'écran des réglages"
```

---

## Task 5: Tooltips génériques + doc + PDF

**Files:**
- Modify: `client/src/columns.js` — `PPF_TIP` (~L29-34)
- Modify: `docs/legende_champs.md` (lignes `ppf_active` / `ppf_usable` + note motifs)
- Modify: `docs/legende_champs.pdf` (régénéré)

- [ ] **Step 1: Rendre les tooltips PPF génériques (`columns.js`)**

Remplacer dans `PPF_TIP` :

```js
  ppf_active: "Annuaire PPF : au moins une ligne au motif C ou P.",
```
par :
```js
  ppf_active: "Annuaire PPF : au moins une ligne à un motif actif configuré (par défaut C ou P).",
```
et :
```js
  ppf_usable: "Annuaire PPF : au moins une même ligne au motif C ou P ET PDP réelle (pdp_fictive = 0).",
```
par :
```js
  ppf_usable: "Annuaire PPF : au moins une même ligne à un motif actif configuré (par défaut C ou P) ET PDP réelle (pdp_fictive = 0).",
```

- [ ] **Step 2: Mettre à jour `docs/legende_champs.md`**

Dans le tableau de la section « 4. Annuaire PPF », remplacer la signification de `ppf_active` par :

> Au moins une ligne à un **motif de présence actif** (ensemble **configurable** dans les réglages, par défaut `C` / `P`).

et celle de `ppf_usable` par :

> Au moins une **même** ligne à un motif actif configuré (défaut `C` / `P`) **ET** PDP réelle (`pdp_fictive = 0`).

Dans la note finale « À propos des colonnes de l'export PPF », remplacer « les motifs C et P sont ceux considérés comme "actifs" » par « les motifs considérés comme "actifs" sont **configurables** dans les réglages (par défaut C et P) ».

- [ ] **Step 3: Régénérer le PDF (charte SFR)**

Mettre à jour le HTML compagnon (même style que la génération initiale — rouge SFR `#E2001A`, en-tête/pied) avec les libellés PPF ci-dessus, puis :

Run:
```bash
weasyprint scratchpad/legende_sfr.html scratchpad/legende_champs.pdf \
  && cp scratchpad/legende_champs.pdf docs/legende_champs.pdf
```
Vérifier visuellement (convertir en PNG et relire la page 4 « Annuaire PPF »).

- [ ] **Step 4: Commit**

```bash
git add client/src/columns.js docs/legende_champs.md docs/legende_champs.pdf
git commit -m "docs(superpopaul): motifs PPF actifs configurables — tooltips + légende + PDF"
```

---

## Récapitulatif de vérification finale

- [ ] `cd client/src-tauri && cargo test` : suite verte (257+ tests + nouveaux).
- [ ] `cargo clippy` : aucun nouveau lint sur les fichiers touchés.
- [ ] YAML `superpopaul.yaml` existant (sans `ppf:`) : se charge, comportement `CP` inchangé (rétro-compat prouvée par test).
- [ ] GUI : réglage saisi, persisté, reflété dans export/rapport ; vide rejeté (validation par l'utilisateur en app).
- [ ] Doc + PDF à jour.
