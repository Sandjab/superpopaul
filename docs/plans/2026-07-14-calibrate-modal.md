# Modale de calibration — plan d'implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Déplacer le banc d'essai du calibrage dans une modale avec cycle explicite : « Arrêter » pendant la mesure, « Retenter / Ignorer / Appliquer » à la fin — et supprimer l'application automatique de la concurrence.

**Architecture:** `calibrate()` gagne un paramètre d'annulation coopérative (`&AtomicBool`, testé en tête de palier) et le rapport un champ `cancelled` ; une commande `cancel_calibration` arme le flag porté par `AppState`. Côté UI, le flux `runCalibration()` ouvre la modale partagée (`#modal`), y construit le banc (div dynamique `id="calibrate-bench"`, CSS existant), et les boutons de fin décident du sort du résultat.

**Tech Stack:** Rust (tauri 2, wiremock), vanilla JS/CSS. Spec : `docs/superpowers/specs/2026-07-14-calibrate-modal-design.md`.

**Conventions :** TDD Rust ; DOM via `h()`/`textContent`, jamais d'innerHTML dynamique ; textes UI en français — « Calibration » (pas « Calibrage ») dans les textes, « Calibrer » reste sur le bouton ; commits `feat(superpopaul): …`. `cargo …` depuis `superpopaul/src-tauri/`.

---

### Task 1: Annulation coopérative dans `calibrate()` (Rust)

**Files:**
- Modify: `superpopaul/src-tauri/src/resolver.rs` — `calibrate()`, `CalibrationReport`, tests de `tests_calibrate`
- Modify: `superpopaul/src-tauri/src/commands.rs` — appel dans `calibrate_api` (flag provisoire, Task 2 branchera le vrai)

- [ ] **Step 1: Écrire les tests d'annulation (RED)** — dans `mod tests_calibrate` (le `use std::sync::atomic::{AtomicBool, Ordering};` est peut-être à ajouter en tête du module de test ; `AtomicBool` est déjà importé dans le fichier) :

```rust
    #[tokio::test]
    async fn calibrate_annule_avant_demarrage_n_emet_rien() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/resolve/batch"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                serde_json::json!({"results": [{"participant_id": "a::1", "exists": true}]}),
            ))
            .mount(&server)
            .await;
        let c = ApiClient::new(&server.uri(), "K", None, None).unwrap();
        let sample: Vec<String> = (0..4).map(|i| format!("0009:{i}")).collect();
        let cancel = std::sync::atomic::AtomicBool::new(true);
        let mut steps = Vec::new();
        let rep = calibrate(&c, &sample, 1, 8, &cancel, |s| steps.push(s)).await;
        assert!(rep.cancelled);
        assert!(steps.is_empty(), "{steps:?}");
        assert_eq!(rep.addr_sent, 0);
    }

    // L'annulation est coopérative : armée pendant le palier 1 (ici au moment
    // de son verdict), elle laisse ce palier se terminer et empêche le suivant.
    #[tokio::test]
    async fn calibrate_annule_apres_un_palier_le_conserve() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/resolve/batch"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                serde_json::json!({"results": [{"participant_id": "a::1", "exists": true}]}),
            ))
            .mount(&server)
            .await;
        let c = ApiClient::new(&server.uri(), "K", None, None).unwrap();
        let sample: Vec<String> = (0..4).map(|i| format!("0009:{i}")).collect();
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let mut steps = Vec::new();
        let rep = calibrate(&c, &sample, 1, 8, &cancel, |s| {
            steps.push(s);
            if !matches!(s, CalibrationStep::Measuring { .. }) {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        })
        .await;
        assert!(rep.cancelled);
        // Palier 1 : Measuring + verdict (Retained garanti : best initial 0.0),
        // puis annulation constatée en tête du palier 2.
        assert_eq!(steps.len(), 2, "{steps:?}");
        assert!(matches!(steps[1], CalibrationStep::Retained { level: 1, .. }), "{steps:?}");
        assert_eq!(rep.best_concurrency, 1); // le partiel reste proposable
    }
```

- [ ] **Step 2: Vérifier l'échec (compilation)** — Run: `cargo test annule 2>&1 | tail -5` — Attendu : E0061 (`calibrate` prend 5 arguments, 6 fournis) et champ `cancelled` inconnu.

- [ ] **Step 3: Implémenter** — `CalibrationReport` gagne le champ (après `addr_sent`) :

```rust
    /// L'utilisateur a demandé l'arrêt (le rapport peut être partiel).
    pub cancelled: bool,
```

`calibrate()` : nouveau paramètre `cancel: &std::sync::atomic::AtomicBool` inséré AVANT `progress`, et en tête de boucle :

```rust
pub async fn calibrate(
    client: &ApiClient,
    sample: &[String],
    batch_size: usize,
    max_concurrency: u32,
    cancel: &std::sync::atomic::AtomicBool,
    mut progress: impl FnMut(CalibrationStep),
) -> CalibrationReport {
    ...
    while level <= max_concurrency {
        // Annulation coopérative : constatée entre les paliers, le palier en
        // cours se termine toujours (son verdict reste exploitable).
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        progress(CalibrationStep::Measuring { level });
        ...
    }
    CalibrationReport {
        best_concurrency: best.0,
        addr_per_s: best.1,
        rate_limited,
        addr_sent,
        cancelled: cancel.load(std::sync::atomic::Ordering::Relaxed),
    }
}
```

- [ ] **Step 4: Adapter les appels existants** — les 4 tests wiremock existants : déclarer `let cancel = std::sync::atomic::AtomicBool::new(false);` et passer `&cancel` en 5e position ; y ajouter `assert!(!rep.cancelled);` là où le rapport est déjà vérifié (au moins dans le test 429). Dans `commands.rs`, `calibrate_api` : passer provisoirement `&std::sync::atomic::AtomicBool::new(false)` via un binding local (`let cancel = …; … &cancel …`) — remplacé à la Task 2.

- [ ] **Step 5: Vérifier** — Run: `cargo test 2>&1 | grep "^test result"` — Attendu : 126 passed, 0 failed, 3 ignored.

- [ ] **Step 6: Commit**

```bash
git add superpopaul/src-tauri/src/resolver.rs superpopaul/src-tauri/src/commands.rs
git commit -m "feat(superpopaul): annulation coopérative du calibrage (cancelled dans le rapport)"
```

---

### Task 2: Flag d'annulation dans `AppState` + commande `cancel_calibration` (Rust)

**Files:**
- Modify: `superpopaul/src-tauri/src/commands.rs` — `AppState`, `calibrate_api`, nouvelle commande, message de garde
- Modify: `superpopaul/src-tauri/src/lib.rs` — enregistrement de la commande

- [ ] **Step 1: AppState** — ajouter le champ (avec `use std::sync::atomic::{AtomicBool, Ordering};` en tête de fichier) :

```rust
    /// Annulation du calibrage en cours — armée par cancel_calibration,
    /// réarmée à false au début de chaque calibrate_api.
    pub calibrate_cancel: Arc<AtomicBool>,
```

et dans `AppState::new` : `calibrate_cancel: Arc::new(AtomicBool::new(false)),`.

- [ ] **Step 2: Commande + branchement** :

```rust
/// Arme l'annulation de la calibration en cours (coopérative : le palier en
/// cours se termine). Sans effet si aucune calibration n'est active.
#[tauri::command]
pub fn cancel_calibration(state: State<'_, AppState>) {
    state.calibrate_cancel.store(true, Ordering::Relaxed);
}
```

Dans `calibrate_api`, juste avant l'appel à `calibrate(...)` : `state.calibrate_cancel.store(false, Ordering::Relaxed);` puis `let cancel = state.calibrate_cancel.clone();` et passer `&cancel` (coercition `&Arc<AtomicBool>` → `&AtomicBool`). Supprimer le binding provisoire de la Task 1.
Au passage (terminologie, spec) : dans `calibration_prerequisites`, le message d'erreur `"Calibrage impossible : il manque {}."` devient `"Calibration impossible : il manque {}."`.

- [ ] **Step 3: lib.rs** — ajouter `commands::cancel_calibration,` dans la liste `generate_handler![…]` (à côté de `commands::calibrate_api`).

- [ ] **Step 4: Vérifier** — `cargo test 2>&1 | grep "^test result"` (126/0/3) et `cargo build 2>&1 | grep -c warning` (0). Pas de test unitaire de la commande (injection Tauri — même dérogation que le relais `calibrate-step` ; le flag lui-même est couvert par les tests de la Task 1).

- [ ] **Step 5: Commit**

```bash
git add superpopaul/src-tauri/src/commands.rs superpopaul/src-tauri/src/lib.rs
git commit -m "feat(superpopaul): commande cancel_calibration (flag AppState)"
```

---

### Task 3: Flux modale côté UI (JS/HTML/CSS)

**Files:**
- Modify: `superpopaul/src/index.html` — suppression du div statique `#calibrate-bench`
- Modify: `superpopaul/src/styles.css` — boutons de modale + ligne d'état
- Modify: `superpopaul/src/app.js` — refonte du flux calibrage en `runCalibration()`

- [ ] **Step 1: index.html** — supprimer entièrement le div statique :

```html
          <div id="calibrate-bench" class="hidden"
               title="Débit mesuré (adr/s) par nombre de sessions. Vert : retenu · rouge : gain < 15 % (arrêt) · jaune : rate-limité (arrêt)."></div>
```

(le CSS `#calibrate-bench`/`.cal-*` reste : il s'appliquera au div créé dynamiquement dans la modale, qui reprend le même `title`.)

- [ ] **Step 2: styles.css** — à la fin :

```css
/* --- Modale de calibration ------------------------------------------------------
   Boutons de fin : Appliquer = action principale (vert plein), Retenter et
   Arrêter en couleur de bordure seulement. */
.modal-btns { display: flex; gap: 8px; justify-content: flex-end; margin-top: 14px; }
.btn-stop { border-color: var(--red); color: var(--red); }
.btn-retry { border-color: var(--blue); color: var(--blue); }
.btn-apply { background: #238636; border-color: var(--green); color: #fff; font-weight: 600; }
#calibrate-status { font-size: 12px; color: var(--muted); margin-top: 6px; }
#calibrate-status.done { color: var(--green); }
```

- [ ] **Step 3: app.js — état du banc et listener** — modifications du bloc existant :

1. `const bench = { cols: new Map(), max: 0, steps: [] };` devient `const bench = { el: null, statusEl: null, cols: new Map(), max: 0, steps: [] };`
2. `benchReset` prend l'élément cible (le div de la modale) et ne gère plus `hidden` :

```js
function benchReset(el) {
  bench.el = el;
  el.replaceChildren();
  bench.cols.clear();
  bench.max = 0;
  bench.steps = [];
}
```

3. Dans le listener `calibrate-step` : remplacer chaque `$("calibrate-bench")` par `bench.el`, avec garde en tête `if (!bench.el) return;` (événement orphelin). Sur un pas `measuring`, mettre à jour la ligne d'état :

```js
    if (bench.statusEl)
      bench.statusEl.textContent = `palier ${s.level} session${s.level > 1 ? "s" : ""} — mesure…`;
```

`benchRescale`, `benchStopReason`, `benchDimLosers` : inchangés.

- [ ] **Step 4: app.js — remplacer le handler du bouton par `runCalibration()`** — le listener actuel de `$("btn-calibrate")` (tout le bloc `async () => { … }`) est remplacé par :

```js
/** Flux complet de calibration dans la modale partagée. L'application de la
 *  concurrence est EXPLICITE (bouton Appliquer) — plus d'écriture automatique. */
async function runCalibration() {
  apiButtons().forEach((b) => { b.disabled = true; });
  syncSettingsForm();
  const out = $("calibrate-result");
  out.textContent = "calibration en cours…";
  const backdrop = $("modal-backdrop");
  let onBackdrop = null;
  let onKeydown = null;
  const cleanup = () => {
    if (onBackdrop) backdrop.removeEventListener("click", onBackdrop);
    if (onKeydown) document.removeEventListener("keydown", onKeydown);
    closeModal();
    bench.el = null;
    bench.statusEl = null;
    apiButtons().forEach((b) => { b.disabled = false; });
  };
  try {
    await invoke("set_config", { cfg: state.config });
    await ensureProxyCreds();
    // La modale ne s'ouvre qu'une fois les prérequis franchis côté UI ; une
    // erreur de garde backend (invoke rejeté) la referme dans le catch.
    const title = h("h3", {}, "Calibration en cours…");
    const benchEl = h("div", {
      id: "calibrate-bench",
      title: "Débit mesuré (adr/s) par nombre de sessions. Vert : retenu · rouge : gain < 15 % (arrêt) · jaune : rate-limité (arrêt).",
    });
    const status = h("div", { id: "calibrate-status" }, "démarrage…");
    const btns = h("div", { class: "modal-btns" });
    modal(title, benchEl, status, btns);
    benchReset(benchEl);
    bench.statusEl = status;
    const stopBtn = h("button", {
      class: "btn-stop",
      onclick: () => {
        stopBtn.disabled = true;
        stopBtn.textContent = "arrêt en cours…";
        invoke("cancel_calibration");
      },
    }, "■ Arrêter");
    btns.append(stopBtn);

    const r = await invoke("calibrate_api");
    benchDimLosers();
    const verdict = `→ ${r.best_concurrency} sessions, ~${Math.round(r.addr_per_s)} adr/s` +
      ` · ${r.addr_sent} adressages consommés` + benchStopReason(r);
    // Un rapport sans aucun palier complet (annulation immédiate) ne doit
    // pas être applicable : best vaudrait (1, 0.0) par défaut.
    const hasComplete = bench.steps.some((s) => s.status !== "measuring");
    if (r.cancelled) {
      title.textContent = "Calibration arrêtée";
      const last = bench.steps[bench.steps.length - 1];
      status.textContent = (last ? `arrêtée au palier ${last.level} · ` : "") +
        `meilleur mesuré : ${r.best_concurrency} sessions, ~${Math.round(r.addr_per_s)} adr/s` +
        ` · ${r.addr_sent} adressages consommés`;
    } else {
      title.textContent = "Calibration terminée";
      status.textContent = verdict;
      status.classList.add("done");
    }
    const finish = (applied) => {
      out.textContent = verdict + (applied ? " — appliquée" : "");
      cleanup();
    };
    const ignore = () => finish(false);
    onBackdrop = (e) => { if (e.target === backdrop) ignore(); };
    onKeydown = (e) => { if (e.key === "Escape") ignore(); };
    backdrop.addEventListener("click", onBackdrop);
    document.addEventListener("keydown", onKeydown);
    const applyBtn = h("button", {
      class: "btn-apply",
      onclick: () => {
        $("api-conc").value = r.best_concurrency;
        $("direct-conc").value = r.best_concurrency; // champs miroirs
        state.config.api.concurrency = r.best_concurrency;
        finish(true);
      },
    }, `✓ Appliquer ${r.best_concurrency} sessions`);
    applyBtn.disabled = !hasComplete;
    btns.replaceChildren(
      h("button", { class: "btn-retry", onclick: () => { cleanup(); runCalibration(); } }, "↻ Retenter"),
      h("button", { onclick: ignore }, "Ignorer"),
      applyBtn,
    );
  } catch (err) {
    cleanup();
    if (err && err.proxyCancelled) out.textContent = "Calibration annulée.";
    else {
      // Échec d'auth proxy probable : re-demander les identifiants au prochain clic.
      if (/407|proxy/i.test(String(err))) proxyCredsGiven = false;
      out.textContent = `❌ ${err}`;
    }
  }
}
$("btn-calibrate").addEventListener("click", runCalibration);
```

Points d'attention :
- Le handler Échap de fermeture du panneau ⚙ (app.js ~329) vérifie déjà que `#modal-backdrop` est caché — enregistré AVANT le nôtre, il voit la modale ouverte et ne ferme pas les réglages ; le nôtre ferme ensuite la modale. Ne rien changer là-bas.
- Pendant la mesure, ni backdrop ni Échap ne sont branchés (modale inerte) — ils ne le sont qu'à l'état terminé/arrêté, où ils valent « Ignorer ».
- `ensureProxyCreds` utilise la même modale AVANT qu'on l'ouvre — séquencement sain, ne pas paralléliser.

- [ ] **Step 5: Vérification syntaxique** — `node --check superpopaul/src/app.js` → code 0 ; `grep -c "calibrate-bench" superpopaul/src/index.html` → 0.

- [ ] **Step 6: Commit**

```bash
git add superpopaul/src/index.html superpopaul/src/styles.css superpopaul/src/app.js
git commit -m "feat(superpopaul): calibration en modale — arrêter, retenter, ignorer, appliquer"
```

---

### Task 4: Validation Chromium

**Files:** aucun code attendu (correctifs éventuels → Task 3). Harnais dans le scratchpad de session.

- [ ] **Step 1: Harnais** — copie de `superpopaul/src/`, stub `__TAURI__` injecté avant `app.js` : `listen` capture les callbacks (`window._listeners`), `invoke("calibrate_api")` rejoue `window._steps` (120 ms/pas) puis résout `window._report` ; `invoke("cancel_calibration")` pose `window._cancelRequested = true` et le rejeu s'interrompt au prochain pas de type verdict en résolvant un rapport `cancelled: true` tronqué ; les autres commandes résolvent `null`. Servir en `python3 -m http.server`, piloter au navigateur.

- [ ] **Step 2: Scénario nominal + Appliquer** — séquence complète (paliers 1→16, `rejected` final), rapport `cancelled: false`. Vérifier : modale ouverte avec titre « Calibration en cours… » puis « Calibration terminée » ; bouton « ■ Arrêter » seul pendant la mesure, puis les 3 boutons (« ✓ Appliquer 8 sessions » vert) ; clic Appliquer → champs `api-conc`/`direct-conc` = 8, modale fermée, `#calibrate-result` = verdict + « — appliquée ».

- [ ] **Step 3: Scénario Arrêter → partiel → Appliquer** — pendant le rejeu, cliquer « ■ Arrêter » (le bouton passe à « arrêt en cours… » désactivé) ; le stub résout un rapport partiel `cancelled: true`. Vérifier : titre « Calibration arrêtée », statut « arrêtée au palier N · meilleur mesuré : … », « Appliquer » actif et porteur de la valeur partielle ; l'appliquer écrit les champs.

- [ ] **Step 4: Scénario Ignorer** — après un run complet, noter la valeur de `api-conc` AVANT, cliquer « Ignorer » : champs INCHANGÉS (c'est le test de la fin de l'auto-application), modale fermée, résumé dans `#calibrate-result` sans « — appliquée ». Vérifier aussi Échap ≡ Ignorer.

- [ ] **Step 5: Scénario garde** — `invoke` rejette (message de prérequis) : la modale ne reste pas ouverte, l'erreur s'affiche dans `#calibrate-result`, boutons réactivés.

- [ ] **Step 6: Rapport** — captures (état en cours, terminé, arrêté) envoyées à l'utilisateur ; nettoyage (serveur arrêté, captures hors du repo) ; rappel : la validation dans l'app Tauri réelle reste dans le backlog commun.
