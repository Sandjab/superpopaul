// Étape 4 : cockpit temps réel. Écoute les événements Rust :
// "telemetry" (4×/s), "run-suspended", "run-resumed", "run-finished".

let running = false;

async function enterRunStep() {
  $("run-title").textContent = state.inputPath ?? "";
  // Pendant un run, revenir sur cet onglet ne relance ni set_config ni
  // analyze_input : le scan CSV + load_map disputerait le Mutex<Store> aux
  // workers, pour re-suggérer un mode qu'on ne peut pas changer.
  if (running) return;
  try {
    await invoke("set_config", { cfg: state.config });
    const s = await invoke("analyze_input");
    $("run-title").textContent = `${state.inputPath} — ${fmt(s.unique)} adressages uniques`;
    suggestMode(s);
  } catch (e) {
    banner("error", `${e}`);
  }
}

/** Aides intelligentes : détection de run incomplet, présélection du mode. */
function suggestMode(s) {
  const known = s.resolved_ok + s.failed + s.stale;
  if (running) return;
  if (s.missing > 0 && known > 0) {
    $("run-mode").value = "reprise";
    banner("warn",
      `Run incomplet détecté : ${fmt(known)}/${fmt(s.unique)} adressages déjà en base. `,
      h("button", { onclick: (e) => {
        e.currentTarget.disabled = true; // pas de double départ pendant les awaits
        hideBanner();
        startRun();
      } }, "Reprendre maintenant"));
  } else if (s.missing === 0 && s.unique > 0) {
    $("run-mode").value = "refresh";
    banner("warn", `Tous les adressages sont déjà en base (${fmt(s.stale)} périmés, ` +
      `${fmt(s.failed)} en échec) — mode refresh présélectionné.`);
  }
}

function modeFromSelect() {
  switch ($("run-mode").value) {
    case "full":          return { mode: "full" };
    case "reprise":       return { mode: "reprise", retry_failures: false };
    case "reprise-retry": return { mode: "reprise", retry_failures: true };
    case "refresh":       return { mode: "refresh", max_age_days: state.config.api.refresh_days };
  }
}

async function startRun() {
  // Garde de ré-entrance pendant les awaits (double clic, bouton de la
  // bannière « Reprendre maintenant ») — convention btn-browse d'app.js.
  const btn = $("btn-start");
  if (btn.disabled) return;
  btn.disabled = true;
  try {
    await invoke("set_config", { cfg: state.config });
    await ensureProxyCreds();
    const total = await invoke("start_run", { mode: modeFromSelect() });
    running = true;
    $("cockpit").classList.remove("hidden");
    $("run-result").classList.add("hidden");
    $("btn-start").classList.add("hidden");
    $("btn-pause").classList.remove("hidden");
    $("btn-stop").classList.remove("hidden");
    hideBanner();
    if (total === 0) banner("warn", "Rien à résoudre dans ce mode — fichier généré directement.");
  } catch (e) {
    banner("error", `${e}`);
  } finally {
    btn.disabled = false;
  }
}
$("btn-start").addEventListener("click", startRun);

$("btn-pause").addEventListener("click", async () => {
  const pausing = $("btn-pause").textContent.includes("Pause");
  try {
    await invoke("pause_run", { paused: pausing });
    $("btn-pause").textContent = pausing ? "▶ Reprendre" : "⏸ Pause";
  } catch (e) {
    banner("error", `${e}`);
  }
});
$("btn-stop").addEventListener("click", () =>
  invoke("stop_run").catch((e) => banner("error", `${e}`)));

// --- Télémétrie -----------------------------------------------------------------
function httpColor(code) {
  if (code === 200) return "var(--green)";
  if (code === 429) return "var(--amber)";
  if (code === 0) return "var(--muted)";
  return code >= 500 ? "var(--red)" : code >= 400 ? "var(--amber)" : "var(--blue)";
}

listen("telemetry", (e) => {
  const s = e.payload;
  const pct = s.total ? (100 * s.done / s.total) : 0;
  $("ring").style.background = `conic-gradient(var(--green) ${pct}%, #21262d ${pct}%)`;
  $("ring-pct").textContent = `${pct.toFixed(pct < 10 ? 1 : 0)}%`;
  $("ring-abs").textContent = `${fmt(s.done)} / ${fmt(s.total)}`;
  $("eta").textContent = s.eta_s != null ? fmtDuration(s.eta_s) : "—";
  $("t-exists").textContent = s.done ? `${(100 * s.exists / s.done).toFixed(1)} %` : "—";
  $("t-ctc").textContent = s.done ? `${(100 * s.ctc / s.done).toFixed(1)} %` : "—";
  $("t-rate").textContent = `${s.req_per_s.toFixed(1)} req/s · ${Math.round(s.addr_per_s)} adr/s`;
  $("t-misc").textContent = `${fmt(s.failed)} échecs`;
  renderHttpBars(s.http);
  const l = s.latency;
  $("latency").textContent = l
    ? `min ${l.min} · moy ${l.mean} · p50 ${l.p50} · p90 ${l.p90} · p99 ${l.p99} · max ${l.max}`
    : "—";
});

function renderHttpBars(http) {
  const entries = Object.entries(http);
  const total = entries.reduce((a, [, n]) => a + n, 0) || 1;
  $("http-bars").replaceChildren(h("div", { class: "hbar" },
    ...entries.map(([code, n]) => {
      const span = h("span", {});
      span.style.width = `${(100 * n / total)}%`;
      span.style.background = httpColor(+code);
      return span;
    })));
  $("http-legend").textContent =
    entries.map(([c, n]) => `${c === "0" ? "réseau" : c}×${fmt(n)}`).join("   ");
}

function fmt(n) { return Number(n).toLocaleString("fr-FR"); }
function fmtDuration(s) {
  if (s < 60) return `${s} s`;
  const m = Math.round(s / 60);
  return m < 60 ? `${m} min` : `${Math.floor(m / 60)} h ${String(m % 60).padStart(2, "0")}`;
}

// --- Suspension / reprise / fin -------------------------------------------------
listen("run-suspended", (e) => {
  const { reason, message, retry_in_s } = e.payload;
  if (reason === "auth_api") {
    // update_api_key lève elle-même la suspension système côté moteur (voir
    // resolver.rs::RunHandle::update_key) — pas de pause_run ici.
    const key = h("input", { type: "password", placeholder: "nouvelle clé API" });
    banner("error", `⛔ ${message} Le traitement est en pause. `, key,
      h("button", { onclick: async () => {
        state.config.api.key = key.value;
        $("api-key").value = key.value;
        await invoke("update_api_key", { key: key.value });
        hideBanner();
      } }, "Reprendre avec cette clé"));
  } else if (reason === "auth_proxy") {
    // set_proxy_creds injecte un nouveau client dans le moteur, qui lève lui-même
    // la suspension système — pas de pause_run ici. Une annulation de la modale
    // (err.proxyCancelled) laisse la bannière affichée pour un nouvel essai.
    banner("error", `⛔ ${message} `, h("button", { onclick: async () => {
      try {
        await ensureProxyCreds(true);
        hideBanner();
      } catch (err) {
        // Annulation : la bannière reste affichée pour un nouvel essai ;
        // toute autre erreur est montrée plutôt que rejetée en silence.
        if (!err.proxyCancelled) banner("error", `${err}`);
      }
    } }, "Ressaisir les identifiants"));
  } else { // server_down
    // pause_run ne lève que la pause UTILISATEUR : pour relancer avant la fin
    // du backoff, il faut la commande dédiée resume_run (même chemin que le
    // timer de reprise automatique du moteur).
    banner("warn",
      `🛑 Serveur indisponible (${message}). Nouvel essai automatique dans ${retry_in_s} s. `,
      h("button", { onclick: () => invoke("resume_run").then(hideBanner) },
        "Réessayer maintenant"));
  }
});
listen("run-resumed", hideBanner);

listen("run-finished", async (e) => {
  const { done, failed, stopped } = e.payload;
  running = false;
  await invoke("clear_run");
  $("btn-start").classList.remove("hidden");
  $("btn-pause").classList.add("hidden");
  $("btn-stop").classList.add("hidden");
  $("btn-pause").textContent = "⏸ Pause";
  const res = $("run-result");
  res.classList.remove("hidden");
  if (stopped) {
    res.replaceChildren(
      `Run arrêté : ${fmt(done)} résolus, rien n'est perdu (mode reprise pour continuer). `,
      h("button", { onclick: writeOutput }, "Générer quand même le fichier"));
  } else {
    res.textContent = `✅ Terminé : ${fmt(done)} résolus, ${fmt(failed)} échecs. Écriture du fichier…`;
    await writeOutput();
  }
});

async function writeOutput() {
  const res = $("run-result");
  try {
    const path = await invoke("generate_output");
    res.textContent = `✅ Fichier de sortie écrit : ${path}`;
  } catch (err) {
    res.textContent = `⚠️ Écriture du fichier impossible : ${err}`;
  }
}
