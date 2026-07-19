// Étape 4 : cockpit temps réel. Écoute les événements Rust :
// "telemetry" (4×/s), "run-suspended", "run-resumed", "run-finished".

let running = false;
// Dernier total connu (via télémétrie) : run-finished ne le porte pas, mais on
// en a besoin pour figer l'anneau à sa valeur finale à la fin du run.
let lastTotal = 0;
// Séries des sparklines (latence, débit). Bornées : au-delà de
// SPARK_MAX_POINTS on décime par 2 et on n'enregistre plus qu'un tick sur
// keepEvery — le graphe couvre ainsi tout le run avec un pas qui grossit,
// sans croître sans fin.
const SPARK_MAX_POINTS = 600;
function makeSparkSeries() {
  return {
    hist: [], keepEvery: 1, tick: 0,
    push(pt) {
      if (++this.tick % this.keepEvery !== 0) return;
      this.hist.push(pt);
      if (this.hist.length > SPARK_MAX_POINTS) {
        this.hist = this.hist.filter((_, i) => i % 2 === 1);
        this.keepEvery *= 2;
      }
    },
    reset() { this.hist = []; this.keepEvery = 1; this.tick = 0; },
  };
}
const latSeries = makeSparkSeries();
const rateSeries = makeSparkSeries();

/** Met à jour l'anneau de progression (fond, %, absolu, ETA). Partagé entre la
 *  télémétrie (4×/s) et run-finished, qui sinon laisserait l'anneau gelé sur le
 *  dernier tick intermédiaire — à concurrence 1, le dernier paquet se termine
 *  sans tick de télémétrie ultérieur, d'où un run complet affiché à 99 %. */
function renderRing(done, total, etaText) {
  const pct = total ? (100 * done / total) : 0;
  $("ring").style.background = `conic-gradient(var(--green) ${pct}%, var(--track) ${pct}%)`;
  $("ring-pct").textContent = `${pct.toFixed(pct < 10 ? 1 : 0)}%`;
  $("ring-abs").textContent = `${fmt(done)} / ${fmt(total)}`;
  $("eta").textContent = etaText;
}

/** État affiché sous l'anneau et bascule ETA ↔ durée + moyenne.
 *  "running" : « en cours » bleu à pulsation douce, ETA normale, moyenne cachée.
 *  "suspended" : « suspendu » orange, durée active et moyenne provisoires
 *  (italique) — l'état vient de la télémétrie (s.halted), source unique :
 *  les reprises anticipées et update_api_key n'émettent pas de Resumed.
 *  "finished" : « terminé », durée et moyenne définitives. */
function setRunState(st) {
  const el = $("ring-state");
  el.classList.toggle("running", st === "running");
  el.classList.toggle("suspended", st === "suspended");
  el.classList.remove("hidden");
  el.textContent = st === "running" ? "en cours"
    : st === "suspended" ? "suspendu" : "terminé";
  const halted = st !== "running";
  $("eta-label").textContent = halted ? "Durée" : "ETA";
  $("eta-line").title = halted
    ? "Durée totale du run, pauses et suspensions exclues."
    : "Temps restant estimé d'après le débit courant.";
  $("avg-rate").classList.toggle("hidden", !halted);
  $("eta-line").classList.toggle("provisional", st === "suspended");
  $("avg-rate").classList.toggle("provisional", st === "suspended");
}
const fmtActive = (s) => (s >= 0.5 ? fmtDuration(Math.round(s)) : "< 1 s");
function renderAvg(done, activeS) {
  const avg = activeS > 0 ? done / activeS : 0;
  $("avg-rate").textContent =
    `≈ ${avg.toLocaleString("fr-FR", { maximumFractionDigits: avg < 10 ? 1 : 0 })} adr/s en moyenne`;
}

/** Aide visible sous le sélecteur de mode — chiffrée dès qu'une analyse du
 *  fichier est disponible (« ce mode résoudra N adressages »), descriptive
 *  sinon. Recalculée au changement de mode, à chaque analyse et à la
 *  fermeture des réglages (l'ancienneté refresh en vient). */
let lastStats = null; // dernier analyze_input (compteurs du pré-run)
function updateRunModeHint() {
  const mode = $("run-mode").value, days = state.config.api.refresh_days;
  const el = $("run-mode-hint");
  if (!lastStats) {
    const hints = {
      "full": "Re-résout tous les adressages, même ceux déjà en cache.",
      "reprise": "Résout uniquement les adressages absents du cache — les résultats existants sont conservés.",
      "reprise-retry": "Résout les absents du cache et re-tente les échecs précédents.",
      "refresh": `Résout les absents, les échecs et les résultats plus vieux que ${days} jours (réglable dans ⚙).`,
    };
    el.textContent = hints[mode] ?? "";
    return;
  }
  const s = lastStats;
  const parts = {
    "full": ["Full re-résoudra la totalité : ", s.unique, " adressages (cache ignoré)."],
    "reprise": ["Reprise résoudra ", s.missing, " adressages jamais résolus — le reste vient du cache."],
    "reprise-retry": ["Reprise + échecs résoudra ", s.missing + s.failed, " adressages (jamais résolus + échecs)."],
    "refresh": ["Refresh résoudra ", s.missing + s.failed + s.stale,
      ` adressages (jamais résolus + échecs + périmés de plus de ${days} jours).`],
  }[mode];
  if (!parts) { el.textContent = ""; return; }
  el.replaceChildren(parts[0], h("b", {}, fmt(parts[1])), parts[2]);
}
$("run-mode").addEventListener("change", updateRunModeHint);
updateRunModeHint();

/** Ligne de stats du pré-run, sous l'en-tête de l'étape. */
function renderRunStats(s) {
  const item = (n, label, alertClass) => {
    const span = h("span", {}, h("b", {}, fmt(n)), ` ${label}`);
    if (alertClass && n > 0) span.classList.add(alertClass);
    return span;
  };
  $("run-stats").replaceChildren(
    item(s.unique, "adressages uniques"),
    item(s.resolved_ok, "déjà résolus"),
    item(s.failed, "en échec", "err"),
    item(s.stale, "périmés", "warn"),
    item(s.missing, "jamais résolus"));
  $("run-stats").classList.remove("hidden");
}

async function enterRunStep() {
  // Titre : le nom du fichier seul, le chemin complet reste en tooltip.
  const t = $("run-title");
  t.textContent = (state.inputPath ?? "").split(/[\\/]/).pop() ?? "";
  t.title = state.inputPath ?? "";
  // Pendant un run, revenir sur cet onglet ne relance ni set_config ni
  // analyze_input : le scan CSV + load_map disputerait le Mutex<Store> aux
  // workers, pour re-suggérer un mode qu'on ne peut pas changer.
  if (running) return;
  // Pas de run en cours : on repart d'un écran propre. Sans ça, le cockpit
  // (ring/tuiles/latences) et run-result garderaient les valeurs du run
  // précédent après un rechargement de config ou un changement de fichier,
  // ce qui est trompeur avant que startRun() ne les réaffiche à jour.
  $("cockpit").classList.add("hidden");
  $("run-result").classList.add("hidden");
  $("run-stats").classList.add("hidden");
  lastStats = null;
  try {
    await invoke("set_config", { cfg: state.config });
    const s = await invoke("analyze_input");
    lastStats = s;
    renderRunStats(s);
    suggestMode(s); // appelle updateRunModeHint() en sortie
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
  updateRunModeHint(); // la présélection change la valeur sans événement change
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
    // La clé n'est plus exigée par une étape du wizard (les réglages sont un
    // panneau ⚙) : c'est ici, au dernier moment, qu'on bloque un run API sans clé.
    if (state.config.api.mode !== "direct" && !state.config.api.key) {
      banner("warn", "Saisis la clé API avant de lancer. ",
        h("button", { onclick: () => { hideBanner(); openSettings(); } }, "Ouvrir les réglages"));
      return;
    }
    await invoke("set_config", { cfg: state.config });
    await ensureProxyCreds();
    const total = await invoke("start_run", { mode: modeFromSelect() });
    running = true;
    lastTotal = total;  // total faisant autorité, avant tout tick de télémétrie
    latSeries.reset();  // les sparklines repartent de zéro à chaque run
    rateSeries.reset();
    $("lat-spark").replaceChildren();
    $("rate-spark").replaceChildren();
    $("cockpit").classList.remove("hidden");
    $("run-result").classList.add("hidden");
    // « running » sous l'anneau, ETA remise à « — » jusqu'au premier tick.
    setRunState("running");
    $("eta").textContent = "—";
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
// Catégories fixes de l'histogramme HTTP, toujours pré-affichées même à zéro
// (l'ordre de déclaration fait foi : premier match gagne, et c'est l'ordre
// d'affichage). « autres » ramasse les codes inattendus (1xx/3xx/2xx hors
// 200) et n'apparaît que si rencontré.
const HTTP_CATS = [
  { label: "200", color: "var(--green)", match: (c) => c === 200,
    help: "Réponses abouties." },
  { label: "429", color: "var(--amber)", match: (c) => c === 429,
    help: "Rate-limit : le moteur réduit la concurrence et retente." },
  { label: "4xx", color: "var(--red)", match: (c) => c >= 400 && c < 500,
    help: "Erreurs client hors 429 : échec définitif du paquet ; 401/407 suspendent le run (clé API / proxy)." },
  { label: "5xx", color: "var(--red)", match: (c) => c >= 500,
    help: "Erreurs serveur : retentées (disjoncteur, reprise automatique)." },
  { label: "réseau", color: "var(--muted)", match: (c) => c === 0,
    help: "Erreurs réseau (connexion, timeout) : retentées." },
  { label: "autres", color: "var(--gold)", match: () => true, onlyIfPresent: true,
    help: "Codes inattendus (1xx, 3xx, 2xx hors 200)." },
];

/** Mini-anneau d'une tuile : % à l'intérieur (en adressages), absolus à côté
 *  en adressages ET en lignes de fichier (le % des lignes couvertes diffère,
 *  un adressage pouvant porter plusieurs lignes). Tant que rien n'est résolu,
 *  tout reste à « — ». Segments optionnels après le vert, absents à 0 :
 *  `laterCount` (« prêt plus tard », vert éteint — pris sur le vert, qui ne
 *  compte plus que les prêts aujourd'hui) puis `warnCount` (« sans verdict »,
 *  orange). */
function renderMiniRing(ring, count, outOf, lineCount, linesOutOf, warnCount = 0, laterCount = 0) {
  const pct = outOf ? (100 * count / outOf) : 0;
  const laterTo = outOf ? pct + (100 * laterCount / outOf) : 0;
  const warnTo = outOf ? laterTo + (100 * warnCount / outOf) : 0;
  const stops = [`var(--green) ${pct}%`];
  if (laterCount > 0) stops.push(`var(--green-later) ${pct}% ${laterTo}%`);
  if (warnCount > 0) stops.push(`var(--amber) ${laterTo}% ${warnTo}%`);
  stops.push(`var(--track) ${warnTo}%`);
  $(`ring-${ring}`).style.background = `conic-gradient(${stops.join(", ")})`;
  $(`t-${ring}`).textContent = outOf ? `${pct.toFixed(1)} %` : "—";
  $(`t-${ring}-abs`).textContent = outOf ? fmt(count) : "—";
  $(`t-${ring}-lines`).textContent = linesOutOf ? fmt(lineCount) : "—";
  $(`t-${ring}-lines-pct`).textContent =
    linesOutOf ? `· ${(100 * lineCount / linesOutOf).toFixed(1)} %` : "";
}

listen("telemetry", (e) => {
  const s = e.payload;
  lastTotal = s.total;
  lastSnap = s; // dernier instantané, réutilisé par « Copier le bilan »
  // Moteur à l'arrêt (pause ou suspension) : « suspendu » + durée active et
  // moyenne provisoires ; sinon « running » + ETA. Ne rien toucher après la
  // fin (running=false) : l'affichage définitif appartient à run-finished.
  if (running) {
    setRunState(s.halted ? "suspended" : "running");
    if (s.halted) renderAvg(s.done, s.active_s);
  }
  renderRing(s.done, s.total,
    s.halted ? fmtActive(s.active_s)
             : s.eta_s != null ? fmtDuration(s.eta_s) : "—");
  renderMiniRing("exists", s.exists, s.done, s.exists_lines, s.done_lines);
  renderMiniRing("ctc", s.ctc, s.done, s.ctc_lines, s.done_lines, s.no_verdict, s.ctc_later);
  // Prêts plus tard : total + prochain palier (première date à venir) —
  // toute la logique vient du snapshot, ici on ne fait qu'afficher.
  $("t-ctc-later").classList.toggle("hidden", !s.ctc_later);
  if (s.ctc_later) {
    $("t-ctc-later-abs").textContent = fmt(s.ctc_later);
    $("t-ctc-later-lines").textContent = fmt(s.ctc_later_lines);
    const next = s.ctc_later_dates[0];
    $("t-ctc-later-next").classList.toggle("hidden", !next);
    if (next) {
      $("t-ctc-later-next-n").textContent = fmt(next.addr);
      $("t-ctc-later-next-d").textContent = fmtDateFr(next.date);
    }
  }
  // Reste à convertir : confirmés dans Peppol sans l'extension France — les
  // « prêts plus tard » et les « sans verdict » (catalogue SMP illisible) en
  // sont exclus et affichés à part, sinon ils gonfleraient le chiffre à
  // convertir ; les supports expirés y restent (dégradation simple).
  $("t-ctc-gap").textContent =
    s.done ? fmt(Math.max(0, s.exists - s.ctc - s.ctc_later - s.no_verdict)) : "—";
  $("t-ctc-gap-lines").textContent =
    s.done ? fmt(Math.max(0, s.exists_lines - s.ctc_lines - s.ctc_later_lines - s.no_verdict_lines)) : "—";
  $("t-ctc-nv").classList.toggle("hidden", !s.no_verdict);
  $("t-ctc-nv-abs").textContent = fmt(s.no_verdict);
  $("t-ctc-nv-lines").textContent = fmt(s.no_verdict_lines);
  $("t-rate").textContent = `${s.req_per_s.toFixed(1)} req/s · ${Math.round(s.addr_per_s)} adr/s`;
  rateSeries.push({ adr: s.addr_per_s });
  renderSpark("rate-spark", rateSeries.hist, [["adr", "var(--gold)"]]);
  // Échecs = info métier, sous l'anneau ; la concurrence reste en télémétrie.
  $("fail-line").textContent = `${fmt(s.failed)} échec${s.failed > 1 ? "s" : ""}`;
  $("fail-line").classList.toggle("zero", s.failed === 0);
  $("t-misc").textContent = s.concurrency_max
    ? `${s.concurrency_allowed} / ${s.concurrency_max}` : "—";
  renderHttpBars(s.http);
  renderTopErrors(s.errors);
  renderPaGrid(s.pa, s.total);
  const l = s.latency;
  $("latency").textContent = l
    ? `min ${l.min} · moy ${l.mean} · p50 ${l.p50} · p90 ${l.p90} · p99 ${l.p99} · max ${l.max}`
    : "—";
  if (l) {
    latSeries.push({ p50: l.p50, p90: l.p90 });
    renderSpark("lat-spark", latSeries.hist,
      [["p90", "var(--amber)"], ["p50", "var(--green)"]]);
  }
  renderLatHist(s.latency_hist);
});

/** Élément SVG (createElement ne suffit pas : il faut l'espace de noms SVG). */
function svgEl(tag, attrs) {
  const el = document.createElementNS("http://www.w3.org/2000/svg", tag);
  for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, v);
  return el;
}

/** Sparkline générique : trace `series` ([clé, couleur]…) sur `hist`.
 *  L'échelle verticale suit le max observé toutes courbes confondues ;
 *  l'horizontale s'étire sur le run. */
function renderSpark(elId, hist, series) {
  if (hist.length < 2) return;
  const W = 300, H = 60, PAD = 2;
  const max = Math.max(...hist.flatMap((p) => series.map(([k]) => p[k])), 1);
  const svg = svgEl("svg", { viewBox: `0 0 ${W} ${H}`, preserveAspectRatio: "none" });
  for (const [key, color] of series) {
    const pts = hist
      .map((p, i) => `${((i * W) / (hist.length - 1)).toFixed(1)},` +
        `${(H - PAD - ((H - 2 * PAD) * p[key]) / max).toFixed(1)}`)
      .join(" ");
    svg.append(svgEl("polyline", {
      points: pts, fill: "none", stroke: color, "stroke-width": "1.5",
      "vector-effect": "non-scaling-stroke",
    }));
  }
  $(elId).replaceChildren(svg);
}

/** Histogramme : une colonne par tranche (bornes fixes côté Rust, dernier
 *  bucket ouvert). Hauteurs relatives au bucket le plus rempli. */
function renderLatHist(hist) {
  if (!hist || !hist.some((b) => b.count)) {
    $("lat-hist").replaceChildren(h("span", { class: "muted" }, "—"));
    return;
  }
  const max = Math.max(...hist.map((b) => b.count));
  const fmtBound = (ms) => (ms >= 1000 ? `${ms / 1000}s` : `${ms}`);
  $("lat-hist").replaceChildren(...hist.map((b, i) => {
    const open = b.le_ms === 0xffffffff; // dernier bucket « au-delà »
    const label = open ? `>${fmtBound(hist[i - 1].le_ms)}`
      : i === 0 ? `≤${fmtBound(b.le_ms)}` : fmtBound(b.le_ms);
    const bar = h("div", { class: "lat-bar" });
    bar.style.height = b.count ? `${Math.max(3, (100 * b.count) / max)}%` : "0";
    return h("div", { class: "lat-bucket", title: `${fmt(b.count)} appel(s)` },
      h("div", { class: "lat-bar-wrap" }, bar),
      h("span", {}, label));
  }));
}

/** Histogramme horizontal par catégorie fixe (HTTP_CATS), longueurs
 *  relatives à la catégorie la plus fréquente. L'infobulle de chaque ligne
 *  porte l'explication et le détail des codes réels agrégés. */
function renderHttpBars(http) {
  const cats = HTTP_CATS.map((c) => ({ ...c, count: 0, detail: [] }));
  for (const [codeStr, n] of Object.entries(http)) {
    const code = +codeStr;
    const cat = cats.find((c) => c.match(code));
    cat.count += n;
    cat.detail.push(`${code === 0 ? "réseau" : code}×${fmt(n)}`);
  }
  const max = Math.max(...cats.map((c) => c.count), 1);
  $("http-hist").replaceChildren(...cats
    .filter((c) => c.count || !c.onlyIfPresent)
    .map((c) => {
      const bar = h("span", { class: "http-bar" });
      bar.style.width = c.count ? `${Math.max(1, (100 * c.count) / max)}%` : "0";
      bar.style.background = c.color;
      const code = h("span", { class: "http-code" }, c.label);
      const count = h("span", { class: "http-count" }, fmt(c.count));
      code.style.color = c.color;
      count.style.color = c.color;
      const title = c.detail.length ? `${c.help}\n${c.detail.join(" · ")}` : c.help;
      return h("div", { class: "http-row", title },
        code, h("div", { class: "http-bar-wrap" }, bar), count);
    }));
}

/** Top erreurs : les 5 motifs d'échec les plus fréquents (le backend borne
 *  déjà à 20 motifs + « (autres) ») ; au-delà de 5, une ligne de synthèse. */
function renderTopErrors(errors) {
  if (!errors.length) {
    $("top-errors").replaceChildren(h("span", { class: "muted" }, "aucun échec"));
    return;
  }
  const rows = errors.slice(0, 5).map((e) =>
    h("div", { class: "err-row" },
      h("span", { class: "err-count" }, fmt(e.count)),
      h("span", { class: "err-name", title: e.name }, e.name)));
  const rest = errors.slice(5);
  if (rest.length) {
    const sum = rest.reduce((a, e) => a + e.count, 0);
    rows.push(h("div", { class: "err-row muted" },
      h("span", { class: "err-count" }, fmt(sum)),
      h("span", { class: "err-name" }, `sur ${rest.length} autres motifs`)));
  }
  $("top-errors").replaceChildren(...rows);
}

/** Carte PA : classement sur 3 colonnes remplies de haut en bas puis de
 *  gauche à droite (rang 1 en haut à gauche). Chaque ligne : rang, nom,
 *  adressages, % du total d'adressages uniques du run. */
function renderPaGrid(pa, total) {
  const grid = $("pa-grid");
  if (!pa.length) {
    grid.replaceChildren(h("span", { class: "muted" }, "—"));
    return;
  }
  const rows = Math.ceil(pa.length / 3);
  const cols = [[], [], []];
  pa.forEach((p, i) => {
    cols[Math.floor(i / rows)].push(h("div", { class: "pa-row" },
      h("span", { class: "pa-rank" }, `${i + 1}`),
      h("span", { class: "pa-name", title: p.name }, p.name),
      h("span", { class: "pa-count" }, fmt(p.count)),
      h("span", { class: "pa-pct" }, total ? `${(100 * p.count / total).toFixed(1)} %` : "—")));
  });
  grid.replaceChildren(...cols.map((c) => h("div", { class: "pa-col" }, ...c)));
}

function fmt(n) { return Number(n).toLocaleString("fr-FR"); }

/** « 2026-09-01 » (clé ISO du snapshot) → « 01/09/2026 ». */
function fmtDateFr(iso) {
  const [y, m, d] = iso.split("-");
  return d && m && y ? `${d}/${m}/${y}` : iso;
}
function fmtDuration(s) {
  if (s < 60) return `${s} s`;
  const m = Math.round(s / 60);
  return m < 60 ? `${m} min` : `${Math.floor(m / 60)} h ${String(m % 60).padStart(2, "0")}`;
}

// --- Fermeture de la fenêtre pendant un run ---------------------------------------
// Enregistrer un listener close-requested transfère la fermeture à l'UI : sans
// preventDefault, l'API JS détruit la fenêtre (d'où core:window:allow-destroy
// dans les capabilities). Pendant un run, on confirme d'abord — les adressages
// déjà résolus sont en base, la reprise se propose au relancement.
const appWindow = window.__TAURI__.window.getCurrentWindow();
appWindow.onCloseRequested((event) => {
  if (!running) return;
  event.preventDefault();
  modal(
    h("h3", {}, "Run en cours"),
    h("p", { class: "muted" },
      "Les adressages déjà résolus sont conservés — le mode Reprise proposera " +
      "de continuer au prochain lancement."),
    h("button", { onclick: () => appWindow.destroy() }, "Quitter quand même"),
    h("button", { onclick: closeModal }, "Continuer le run"),
  );
}).catch((e) => banner("error", `Garde de fermeture non installée : ${e}`));

// --- Suspension / reprise / fin -------------------------------------------------
listen("run-suspended", (e) => {
  const { reason, message, retry_in_s } = e.payload;
  if (reason === "auth_api") {
    // update_api_key lève elle-même la suspension système côté moteur (voir
    // resolver.rs::RunHandle::update_client, appelé via
    // commands::update_api_key) — pas de pause_run ici.
    const key = h("input", { type: "password", placeholder: "nouvelle clé API" });
    banner("error", `⛔ ${message} Le traitement est en pause. `, key,
      h("button", { onclick: async () => {
        state.config.api.key = key.value;
        $("api-key").value = key.value;
        try {
          await invoke("update_api_key", { key: key.value });
          hideBanner();
          // Réglages auto-persistés : la nouvelle clé doit survivre au
          // redémarrage, pas seulement au run en cours.
          try {
            await invoke("save_settings", { settings: currentSettings() });
          } catch (err) {
            banner("warn", `Clé appliquée au run, mais non enregistrée : ${err}`);
          }
        } catch (err) {
          banner("error", `${err}`);
        }
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
      h("button", { onclick: () => invoke("resume_run").then(hideBanner)
        .catch((e) => banner("error", `${e}`)) },
        "Réessayer maintenant"));
  }
});
listen("run-resumed", hideBanner);

let lastRun = null; // bilan du dernier run (pour « Copier le bilan »)
let lastSnap = null; // dernier Snapshot de télémétrie (pourcentages du bilan)
const statPair = (v, label) => h("span", {}, h("b", {}, v), ` ${label}`);

/** Bilan d'une ligne dans le presse-papiers, pour un mail ou un ticket. */
function copyReport(btn) {
  const name = (state.inputPath ?? "").split(/[\\/]/).pop();
  const r = lastRun, s = lastSnap;
  const pct = (a, b) => (b ? `${(100 * a / b).toFixed(1).replace(".", ",")} %` : "—");
  const next = s?.ctc_later ? s.ctc_later_dates[0] : null;
  const text = `Super Popaul — ${name} : ${fmt(r.done)} résolus, ${fmt(r.failed)} échecs` +
    (s ? `, ${pct(s.exists, s.done)} dans Peppol, ${pct(s.ctc, s.done)} prêts aujourd'hui (extension France)` : "") +
    (next ? `, +${fmt(next.addr)} dès le ${fmtDateFr(next.date)}` : "") +
    `, durée ${fmtActive(r.active_s)}.`;
  navigator.clipboard.writeText(text).then(
    () => {
      btn.textContent = "Copié ✓";
      setTimeout(() => { btn.textContent = "Copier le bilan"; }, 1500);
    },
    () => { btn.textContent = "Copie impossible"; });
}

listen("run-finished", async (e) => {
  const { done, failed, stopped, active_s } = e.payload;
  running = false;
  lastRun = { done, failed, active_s };
  // Fige l'anneau sur sa valeur finale : un run complet passe ainsi à 100 %
  // (done == total) au lieu de rester sur le dernier tick de télémétrie ; un
  // run arrêté reflète sa progression réelle. À la place de l'ETA : la durée
  // active du run (pauses et suspensions exclues), et en dessous la moyenne.
  renderRing(done, lastTotal || done, fmtActive(active_s));
  setRunState("finished");
  renderAvg(done, active_s);
  await invoke("clear_run");
  $("btn-start").classList.remove("hidden");
  $("btn-pause").classList.add("hidden");
  $("btn-stop").classList.add("hidden");
  $("btn-pause").textContent = "⏸ Pause";
  const res = $("run-result");
  res.classList.remove("hidden", "stopped", "failed");
  const avg = active_s > 0 ? done / active_s : 0;
  if (stopped) {
    res.classList.add("stopped");
    res.replaceChildren(
      h("p", { class: "result-title" }, "Run arrêté — rien n'est perdu"),
      h("div", { class: "result-stats" },
        statPair(fmt(done), "résolus"),
        statPair(fmtActive(active_s), "durée active")),
      h("div", { class: "result-file" },
        h("span", { class: "path" }, "mode reprise pour continuer plus tard"),
        h("button", { onclick: writeOutput }, "Générer quand même le fichier"),
        h("button", { onclick: (ev) => exportReport(ev.currentTarget) }, "Rapport HTML")));
  } else {
    res.replaceChildren(
      h("p", { class: "result-title" }, "✅ Run terminé"),
      h("div", { class: "result-stats" },
        statPair(fmt(done), "résolus"),
        statPair(fmt(failed), "échecs"),
        statPair(fmtActive(active_s), "durée active"),
        statPair(`≈ ${Math.round(avg)} adr/s`, "en moyenne")),
      h("div", { class: "result-file" },
        h("span", { class: "path" }, "écriture du fichier…")));
    await writeOutput();
  }
});

/** Écrit le rapport HTML client du dernier run (commande export_report) puis
 *  le révèle dans le dossier. Le résultat vit dans le bouton (✓ / message),
 *  comme pour « Copier le bilan ». */
async function exportReport(btn) {
  btn.disabled = true;
  const label = btn.textContent;
  try {
    const path = await invoke("export_report");
    btn.textContent = "Rapport ✓";
    window.__TAURI__.opener?.revealItemInDir(path);
  } catch (err) {
    btn.textContent = "Rapport impossible";
    btn.title = String(err);
  }
  setTimeout(() => { btn.disabled = false; btn.textContent = label; }, 1500);
}

async function writeOutput() {
  const res = $("run-result");
  const row = res.querySelector(".result-file");
  try {
    const path = await invoke("generate_output");
    const name = path.split(/[\\/]/).pop();
    const dir = path.slice(0, path.length - name.length);
    res.classList.remove("failed");
    row.replaceChildren(
      h("span", {}, "📄 ", h("b", {}, name)),
      h("span", { class: "path", title: path }, dir),
      h("button", { onclick: () => window.__TAURI__.opener?.revealItemInDir(path) },
        "Afficher dans le dossier"),
      h("button", { onclick: (ev) => copyReport(ev.currentTarget) }, "Copier le bilan"),
      h("button", { onclick: (ev) => exportReport(ev.currentTarget) }, "Rapport HTML"));

    const hasDir = state.config.output.columns.some(
      (c) => c.source === "peppol" && c.field === "in_directory");
    if (hasDir) {
      const st = await invoke("directory_status").catch(() => null);
      if (!st)
        banner("warn",
          "La colonne « annuaire Peppol » est vide : l'annuaire n'a pas été chargé (onglet Fichiers).");
    }
  } catch (err) {
    res.classList.add("failed");
    row.replaceChildren(
      h("span", { class: "path" }, `⚠️ Écriture du fichier impossible : ${err}`),
      h("button", { onclick: writeOutput }, "Réessayer"));
  }
}
