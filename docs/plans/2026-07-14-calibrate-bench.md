# Banc d'essai du calibrage — plan d'implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Pendant le calibrage API de Super Popaul, afficher dans le panneau ⚙ un graphe en barres qui se construit palier par palier (bleu = mesure, vert = retenu, rouge = gain < 15 %, jaune = 429), à la place du texte statique.

**Architecture:** `resolver::calibrate()` gagne une closure de progression appelée 2× par palier ; `commands::calibrate_api` relaie chaque pas en `app.emit("calibrate-step", …)` (même pattern que `telemetry`) ; `app.js` écoute et construit le DOM via `h()` dans un bloc `#calibrate-bench`. Aucune logique métier côté UI (le calcul du « +x % » affiché est du formatage de présentation).

**Tech Stack:** Rust (tauri 2, serde, wiremock pour les tests), vanilla JS/CSS. Spec : `docs/superpowers/specs/2026-07-14-calibrate-bench-design.md`.

**Conventions à respecter (CLAUDE.md du sous-projet) :** TDD pour toute logique Rust ; DOM via `h()`/`textContent`, jamais d'innerHTML avec données dynamiques ; texte UI en français ; commits `feat(superpopaul): …`. Couleurs : réutiliser les variables de `styles.css` (`--green: #3fb950`, `--blue: #58a6ff`, `--amber: #d29922`, `--red: #f85149`, `--border: #30363d`, `--muted: #8b949e`) — PAS les hex de la maquette.

**Commandes :** tous les `cargo …` se lancent depuis `superpopaul/src-tauri/`.

---

### Task 1: Enum `CalibrationStep` + verdict pur (Rust)

**Files:**
- Modify: `superpopaul/src-tauri/src/resolver.rs` (près de `CalibrationReport`, ~ligne 688)
- Modify: `docs/superpowers/specs/2026-07-14-calibrate-bench-design.md` (retrait de `Measured`)
- Test: module `tests_calibrate` du même fichier (~ligne 1292)

- [ ] **Step 1: Écrire les tests du verdict (RED)**

Dans `mod tests_calibrate` de `resolver.rs`, ajouter :

```rust
    // Le verdict d'un palier encode la sémantique des couleurs de la spec :
    // jaune (429) prime sur tout, vert = gain > 15 %, rouge = le reste.
    #[test]
    fn verdict_retenu_si_gain_suffisant() {
        match palier_verdict(4, 50.0, 40.0, false) {
            CalibrationStep::Retained { level: 4, .. } => {}
            other => panic!("attendu Retained niveau 4, obtenu {other:?}"),
        }
    }

    #[test]
    fn verdict_rejete_si_gain_insuffisant() {
        // 45 < 40 × 1,15 = 46 : c'est le palier qui arrête le calibrage.
        match palier_verdict(16, 45.0, 40.0, false) {
            CalibrationStep::Rejected { level: 16, .. } => {}
            other => panic!("attendu Rejected niveau 16, obtenu {other:?}"),
        }
    }

    #[test]
    fn verdict_jaune_prime_sur_le_gain() {
        // Un 429 pendant la mesure rend le débit non fiable, même excellent.
        match palier_verdict(8, 100.0, 10.0, true) {
            CalibrationStep::RateLimited { level: 8, .. } => {}
            other => panic!("attendu RateLimited niveau 8, obtenu {other:?}"),
        }
    }
```

- [ ] **Step 2: Vérifier l'échec (compilation)**

Run: `cargo test verdict 2>&1 | tail -5`
Expected: erreurs E0425/E0433 — `palier_verdict` et `CalibrationStep` n'existent pas.

- [ ] **Step 3: Implémenter enum + fonction**

Dans `resolver.rs`, juste au-dessus de `CalibrationReport` :

```rust
/// Un pas de progression du calibrage, sérialisé tel quel vers l'UI
/// (événement `calibrate-step`). Tag interne : `{"status":"measuring","level":1}`.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CalibrationStep {
    Measuring { level: u32 },
    Retained { level: u32, addr_per_s: f64 },
    Rejected { level: u32, addr_per_s: f64 },
    RateLimited { level: u32, addr_per_s: f64 },
}

/// Verdict d'un palier mesuré. Le jaune (RateLimited) prime sur le rouge :
/// une mesure traversée par un 429 n'est pas fiable, quel que soit son gain.
fn palier_verdict(
    level: u32,
    throughput: f64,
    best_throughput: f64,
    rate_limited: bool,
) -> CalibrationStep {
    if rate_limited {
        CalibrationStep::RateLimited { level, addr_per_s: throughput }
    } else if throughput > best_throughput * 1.15 {
        CalibrationStep::Retained { level, addr_per_s: throughput }
    } else {
        CalibrationStep::Rejected { level, addr_per_s: throughput }
    }
}
```

Le seuil `* 1.15` doit rester identique à celui de la boucle de `calibrate()` (il y sera factorisé au Task 2).

- [ ] **Step 4: Vérifier le passage**

Run: `cargo test verdict 2>&1 | tail -5`
Expected: `3 passed`. (Un warning `dead_code` sur `palier_verdict` est attendu — il disparaît au Task 2.)

- [ ] **Step 5: Amender la spec (retrait de `Measured`)**

Dans `docs/superpowers/specs/2026-07-14-calibrate-bench-design.md`, remplacer :
`{ level, addr_per_s, status: Measured | Retained | Rejected | RateLimited }`
par :
`{ level, addr_per_s, status: Retained | Rejected | RateLimited }`
et remplacer « plafond → dernier pas `Measured`/`Retained`, pas de pas d'arrêt » par « plafond → dernier pas `Retained`, pas de pas d'arrêt ».
(L'état « gris = mesuré, dépassé » est un état de rendu dérivé côté UI, pas un statut backend.)

- [ ] **Step 6: Commit**

```bash
git add superpopaul/src-tauri/src/resolver.rs docs/superpowers/specs/2026-07-14-calibrate-bench-design.md
git commit -m "feat(superpopaul): CalibrationStep et verdict de palier (spec banc d'essai)"
```

---

### Task 2: `calibrate()` émet les pas de progression (Rust)

**Files:**
- Modify: `superpopaul/src-tauri/src/resolver.rs` — fonction `calibrate()` (~ligne 697) et les 2 tests wiremock existants de `tests_calibrate`
- Modify: `superpopaul/src-tauri/src/commands.rs` — appel dans `calibrate_api` (~ligne 270), provisoirement `|_| {}`

- [ ] **Step 1: Écrire les tests de séquence (RED)**

Dans `mod tests_calibrate`, ajouter un helper et deux tests :

```rust
    fn level_of(s: &CalibrationStep) -> u32 {
        match s {
            CalibrationStep::Measuring { level, .. }
            | CalibrationStep::Retained { level, .. }
            | CalibrationStep::Rejected { level, .. }
            | CalibrationStep::RateLimited { level, .. } => *level,
        }
    }

    // La cadence des mocks n'est pas déterministe : on vérifie la STRUCTURE
    // de la séquence (paires Measuring→verdict, niveaux qui doublent), pas
    // le nombre exact de paliers.
    #[tokio::test]
    async fn calibrate_emet_measuring_puis_verdict_par_palier() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/resolve/batch"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(std::time::Duration::from_millis(50))
                    .set_body_json(serde_json::json!({"results": [
                        {"participant_id": "a::1", "exists": true}
                    ]})),
            )
            .mount(&server)
            .await;
        let c = ApiClient::new(&server.uri(), "K", None, None).unwrap();
        let sample: Vec<String> = (0..8).map(|i| format!("0009:{i}")).collect();
        let mut steps = Vec::new();
        let rep = calibrate(&c, &sample, 1, 8, |s| steps.push(s)).await;

        assert!(steps.len() >= 2 && steps.len() % 2 == 0, "paires attendues : {steps:?}");
        let mut expected = 1u32;
        for pair in steps.chunks(2) {
            assert!(matches!(pair[0], CalibrationStep::Measuring { .. }), "{pair:?}");
            assert!(!matches!(pair[1], CalibrationStep::Measuring { .. }), "{pair:?}");
            assert_eq!(level_of(&pair[0]), expected);
            assert_eq!(level_of(&pair[1]), expected);
            expected *= 2;
        }
        // Cohérence avec le rapport : si le dernier verdict est Retained
        // (arrêt par plafond), c'est lui le gagnant.
        if let Some(CalibrationStep::Retained { level, .. }) = steps.last() {
            assert_eq!(*level, rep.best_concurrency);
        }
    }

    #[tokio::test]
    async fn calibrate_stoppe_en_jaune_sur_429() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/resolve/batch"))
            .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "1"))
            .mount(&server)
            .await;
        let c = ApiClient::new(&server.uri(), "K", None, None).unwrap();
        let sample: Vec<String> = (0..4).map(|i| format!("0009:{i}")).collect();
        let mut steps = Vec::new();
        let rep = calibrate(&c, &sample, 1, 8, |s| steps.push(s)).await;

        assert!(rep.rate_limited);
        match steps.last() {
            Some(CalibrationStep::RateLimited { level: 1, .. }) => {}
            other => panic!("attendu RateLimited niveau 1, obtenu {other:?}"),
        }
    }
```

- [ ] **Step 2: Vérifier l'échec (compilation)**

Run: `cargo test calibrat 2>&1 | tail -5`
Expected: E0061 — `calibrate` prend 4 arguments, 5 fournis.

- [ ] **Step 3: Modifier `calibrate()`**

Nouvelle signature et boucle (le corps de mesure ne change pas ; le verdict est émis AVANT la mise à jour de `best`, avec l'ancien meilleur — même comparaison qu'aujourd'hui, factorisée dans `palier_verdict`) :

```rust
/// Salves à concurrence croissante (1, 2, 4, … ≤ max) : mesure le débit de
/// chaque palier, s'arrête au premier 429 ou quand le gain devient < 15 %.
/// `progress` est appelée 2× par palier : Measuring puis verdict.
pub async fn calibrate(
    client: &ApiClient,
    sample: &[String],
    batch_size: usize,
    max_concurrency: u32,
    mut progress: impl FnMut(CalibrationStep),
) -> CalibrationReport {
    let mut best = (1u32, 0.0f64);
    let mut rate_limited = false;
    let mut addr_sent = 0u64;
    let mut level = 1u32;
    while level <= max_concurrency {
        progress(CalibrationStep::Measuring { level });
        let t0 = std::time::Instant::now();
        // … boucle de spawn + collecte INCHANGÉE (chunks, addr_sent, ok, rate_limited) …
        let throughput = ok as f64 / t0.elapsed().as_secs_f64().max(0.001);
        let step = palier_verdict(level, throughput, best.1, rate_limited);
        progress(step);
        match step {
            CalibrationStep::Retained { .. } => best = (level, throughput),
            _ => break, // Rejected ou RateLimited : le calibrage s'arrête là
        }
        level *= 2;
    }
    CalibrationReport {
        best_concurrency: best.0,
        addr_per_s: best.1,
        rate_limited,
        addr_sent,
    }
}
```

Attention à l'équivalence de comportement : l'ancien code faisait `if throughput > best.1 * 1.15 { best = … } else { break; } if rate_limited { break; }`. Un palier à la fois gagnant ET rate-limité mettait à jour `best` puis s'arrêtait ; le nouveau code s'arrête SANS le retenir (verdict jaune ⇒ mesure non fiable, décision spec). C'est le SEUL changement de comportement autorisé ; il est couvert par `verdict_jaune_prime_sur_le_gain`.

- [ ] **Step 4: Adapter les appels existants**

Les 2 tests wiremock existants (`calibrate_renvoie_un_debit_et_une_concurrence`, `calibrate_compte_les_adressages_envoyes`) : ajouter l'argument `|_| {}` en 5e position. Dans `commands.rs`, `calibrate_api` : idem (`|_| {}` provisoire, remplacé au Task 3).

- [ ] **Step 5: Vérifier le passage + suite complète**

Run: `cargo test 2>&1 | grep "^test result"`
Expected: tous verts (≥ 123 passed, 3 ignored), 0 failed.

- [ ] **Step 6: Commit**

```bash
git add superpopaul/src-tauri/src/resolver.rs superpopaul/src-tauri/src/commands.rs
git commit -m "feat(superpopaul): calibrate() émet un pas de progression par palier"
```

---

### Task 3: Relais Tauri `calibrate-step` (Rust)

**Files:**
- Modify: `superpopaul/src-tauri/src/commands.rs` — signature de `calibrate_api` (~ligne 251) et closure

- [ ] **Step 1: Brancher l'émission**

`start_run` (ligne 302) montre le pattern : paramètre `app: AppHandle` injecté par Tauri (l'appel JS `invoke("calibrate_api")` reste sans argument). Modifier :

```rust
#[tauri::command]
pub async fn calibrate_api(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CalibrationReport, String> {
    // … corps inchangé (garde mode direct, prérequis, scan CSV) …
    Ok(calibrate(
        &client,
        &sample,
        cfg.api.batch_size as usize,
        cfg.api.concurrency.max(16),
        |step| {
            let _ = app.emit("calibrate-step", &step);
        },
    )
    .await)
}
```

`AppHandle` et le trait `Emitter` sont déjà importés (utilisés par `start_run`, `app.emit("telemetry", …)` ligne ~361) — vérifier le `use` en tête de fichier, ne rien dupliquer.

- [ ] **Step 2: Vérifier compilation + tests**

Run: `cargo test 2>&1 | grep -E "^test result|error"`
Expected: mêmes comptes qu'au Task 2, 0 failed. (Pas de test unitaire du relais : il exige un runtime Tauri ; la séquence est déjà couverte au niveau `calibrate()`, le rendu le sera au Task 6.)

- [ ] **Step 3: Commit**

```bash
git add superpopaul/src-tauri/src/commands.rs
git commit -m "feat(superpopaul): calibrate_api relaie calibrate-step vers l'UI"
```

---

### Task 4: Conteneur + styles du banc (HTML/CSS)

**Files:**
- Modify: `superpopaul/src/index.html` — après le `<p>` du bouton Calibrer (ligne ~157, dans `#api-fields`)
- Modify: `superpopaul/src/styles.css` — à la fin du fichier

- [ ] **Step 1: Ajouter le conteneur**

Dans `index.html`, juste après la ligne du bouton `btn-calibrate` (le `</p>` qui contient `<span id="calibrate-result">`), toujours DANS `#api-fields` :

```html
          <div id="calibrate-bench" class="hidden"
               title="Débit mesuré (adr/s) par nombre de sessions. Vert : retenu · rouge : gain < 15 % (arrêt) · jaune : rate-limité (arrêt)."></div>
```

- [ ] **Step 2: Ajouter les styles**

À la fin de `styles.css` :

```css
/* --- Banc d'essai du calibrage (panneau ⚙) ------------------------------------
   Une colonne par palier : valeur (adr/s), barre, label (nb de sessions).
   Hauteurs pilotées par app.js (normalisées sur le meilleur débit vu). */
#calibrate-bench { display: flex; align-items: flex-end; gap: 8px; height: 84px; margin: 2px 0 8px 2px; }
.cal-col { display: flex; flex-direction: column; align-items: center; justify-content: flex-end; gap: 3px; width: 34px; }
.cal-bar { width: 100%; border-radius: 3px 3px 0 0; background: var(--border); transition: height .3s ease; }
.cal-bar.measuring { background: var(--blue); height: 6px; animation: cal-pulse 1s ease-in-out infinite; }
@keyframes cal-pulse { 50% { opacity: .45; } }
.cal-bar.win { background: var(--green); }
.cal-bar.reject { background: var(--red); }
.cal-bar.ratelimited { background: var(--amber); }
.cal-bar.dim { opacity: .35; }
.cal-val, .cal-lab { font-size: 10px; color: var(--muted); }
.cal-val { height: 12px; }
.cal-col.win .cal-val { color: var(--green); }
.cal-col.reject .cal-val { color: var(--red); }
.cal-col.ratelimited .cal-val { color: var(--amber); }
```

- [ ] **Step 3: Vérification rapide**

Run: `node --check superpopaul/src/app.js && grep -c "calibrate-bench" superpopaul/src/index.html superpopaul/src/styles.css`
Expected: `1` dans chaque fichier (le vrai contrôle visuel est au Task 6).

- [ ] **Step 4: Commit**

```bash
git add superpopaul/src/index.html superpopaul/src/styles.css
git commit -m "feat(superpopaul): conteneur et styles du banc d'essai de calibrage"
```

---

### Task 5: Rendu du banc + raison d'arrêt (JS)

**Files:**
- Modify: `superpopaul/src/app.js` — section « Réglages : test API et calibrage » (~ligne 345)

- [ ] **Step 1: État + listener `calibrate-step`**

Insérer AVANT le handler `$("btn-calibrate")` :

```js
// --- Banc d'essai du calibrage : une colonne par palier, hauteurs re-échelonnées
// sur le meilleur débit vu (le backend n'envoie que des adr/s absolus).
const bench = { cols: new Map(), max: 0, steps: [] };

function benchReset() {
  $("calibrate-bench").replaceChildren();
  bench.cols.clear();
  bench.max = 0;
  bench.steps = [];
  $("calibrate-bench").classList.remove("hidden");
}

function benchRescale() {
  for (const { bar } of bench.cols.values()) {
    const v = Number(bar.dataset.adr || 0);
    if (v > 0 && bench.max > 0)
      bar.style.height = `${Math.max(4, Math.round((v / bench.max) * 52))}px`;
  }
}

listen("calibrate-step", (e) => {
  const s = e.payload;
  bench.steps.push(s);
  if (s.status === "measuring") {
    const val = h("span", { class: "cal-val" }, "");
    const bar = h("div", { class: "cal-bar measuring" });
    const col = h("div", { class: "cal-col" }, val, bar,
      h("span", { class: "cal-lab" }, String(s.level)));
    bench.cols.set(s.level, { col, bar, val });
    $("calibrate-bench").append(col);
    return;
  }
  const entry = bench.cols.get(s.level);
  if (!entry) return;
  entry.bar.classList.remove("measuring");
  entry.bar.dataset.adr = String(s.addr_per_s);
  entry.val.textContent = String(Math.round(s.addr_per_s));
  if (s.addr_per_s > bench.max) bench.max = s.addr_per_s;
  if (s.status === "retained") {
    // Le vert bascule : l'ancien meilleur redevient gris.
    for (const { col, bar } of bench.cols.values()) {
      col.classList.remove("win");
      bar.classList.remove("win");
    }
    entry.col.classList.add("win");
    entry.bar.classList.add("win");
  } else if (s.status === "rejected") {
    entry.col.classList.add("reject");
    entry.bar.classList.add("reject");
  } else if (s.status === "rate_limited") {
    entry.col.classList.add("ratelimited");
    entry.bar.classList.add("ratelimited");
  }
  benchRescale();
});

/** Raison d'arrêt pour le verdict texte — formatage de présentation uniquement. */
function benchStopReason(r) {
  const last = bench.steps[bench.steps.length - 1];
  if (!last) return r.rate_limited ? " (clé rate-limitée)" : "";
  if (last.status === "rate_limited") return ` (${last.level} : rate-limité, arrêt)`;
  if (last.status === "rejected") {
    const gain = r.addr_per_s > 0
      ? Math.round((last.addr_per_s / r.addr_per_s - 1) * 100) : 0;
    return ` (${last.level} : ${gain >= 0 ? "+" : ""}${gain} % < 15 %, arrêt)`;
  }
  return ""; // arrêt par plafond : rien à expliquer
}

function benchDimLosers() {
  for (const { bar } of bench.cols.values()) {
    if (!bar.classList.contains("win") && !bar.classList.contains("reject")
        && !bar.classList.contains("ratelimited")) bar.classList.add("dim");
  }
}
```

- [ ] **Step 2: Brancher dans le handler du bouton**

Dans le listener existant de `$("btn-calibrate")` :
1. Après `await ensureProxyCreds();` (et AVANT `invoke("calibrate_api")`), ajouter `benchReset();` — ainsi une erreur de prérequis n'affiche pas un banc vide.
2. Remplacer la construction du verdict :

```js
    const r = await invoke("calibrate_api");
    $("api-conc").value = r.best_concurrency;
    $("direct-conc").value = r.best_concurrency; // champs miroirs
    state.config.api.concurrency = r.best_concurrency;
    benchDimLosers();
    out.textContent = `→ ${r.best_concurrency} sessions, ~${Math.round(r.addr_per_s)} adr/s` +
      ` · ${r.addr_sent} adressages consommés` + benchStopReason(r);
```

(le suffixe `(r.rate_limited ? " (clé rate-limitée)" : "")` disparaît — `benchStopReason` le remplace, avec repli sur l'ancien texte si aucun pas n'est arrivé).
3. Dans le `catch`, ajouter en première ligne : `$("calibrate-bench").classList.add("hidden");`

- [ ] **Step 3: Vérification syntaxique**

Run: `node --check superpopaul/src/app.js`
Expected: sortie vide (code 0).

- [ ] **Step 4: Commit**

```bash
git add superpopaul/src/app.js
git commit -m "feat(superpopaul): rendu du banc d'essai au fil des calibrate-step"
```

---

### Task 6: Validation Chromium (rendu réel)

**Files:**
- Aucune modification de code attendue (des correctifs éventuels retournent aux Tasks 4/5)
- Travail dans le scratchpad de session (copie de `superpopaul/src/`)

- [ ] **Step 1: Monter le harnais**

Protocole habituel du repo (mémoire « pièges de session ») : copier `superpopaul/src/` dans le scratchpad, injecter avant `app.js` un `tauri-stub.js` qui expose `window.__TAURI__` avec :
- `event.listen(name, cb)` qui CAPTURE les callbacks dans `window._listeners[name] = cb` (et retourne `Promise.resolve(() => {})`) ;
- `core.invoke("calibrate_api")` qui rejoue une séquence scriptée avec `setTimeout` en appelant `window._listeners["calibrate-step"]({ payload: step })`, puis résout le rapport ;
- les autres commandes (`set_config`, `load_settings`…) résolvant `null`.
Servir avec `python3 -m http.server` et piloter au navigateur (Playwright).

- [ ] **Step 2: Scénario nominal (arrêt rouge)**

Séquence scriptée : paliers 1 (9 adr/s), 2 (18), 4 (36), 8 (54) tous `retained`, puis 16 `rejected` (56) ; rapport `{best_concurrency: 8, addr_per_s: 54, rate_limited: false, addr_sent: 1550}`.
Vérifier programmatiquement après résolution :
- 5 `.cal-col` ; barre 8 a la classe `win`, barre 16 la classe `reject`, barres 1/2/4 la classe `dim` ;
- hauteur barre 16 > hauteur barre 8 > … > barre 1 (normalisation) ;
- `#calibrate-result` contient `(16 : +4 % < 15 %, arrêt)`.
Capture d'écran de la phase mesure (barre bleue qui pulse) et du verdict.

- [ ] **Step 3: Scénario 429 (arrêt jaune)**

Séquence : palier 1 `retained` (20), palier 2 `rate_limited` (5) ; rapport `{best_concurrency: 1, addr_per_s: 20, rate_limited: true, addr_sent: 150}`.
Vérifier : barre 2 en classe `ratelimited`, `#calibrate-result` contient `(2 : rate-limité, arrêt)`.

- [ ] **Step 4: Scénario garde (pas de banc vide)**

`invoke("calibrate_api")` rejette avec le message de prérequis → vérifier que `#calibrate-bench` a la classe `hidden` et que `#calibrate-result` affiche l'erreur.

- [ ] **Step 5: Rapport final**

Envoyer les captures à l'utilisateur ; rappeler que la validation dans l'app Tauri réelle (run `tauri dev`) reste dans le backlog commun du lot UI. Nettoyer : captures hors du repo, serveur arrêté.

- [ ] **Step 6: Commit final (si correctifs)**

```bash
git add -A superpopaul/src docs/superpowers
git commit -m "fix(superpopaul): ajustements du banc d'essai après validation Chromium"
```
(Uniquement si les steps 2–4 ont exigé des retouches ; sinon rien à committer.)
