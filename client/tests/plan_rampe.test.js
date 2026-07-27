// Rampe manuelle : volumes saisis run par run. Le moteur les rend verbatim et
// compte 0 pour tout run absent de la map — l'UI doit donc lister TOUS les runs
// retenus, et ne jamais perdre une saisie.

const test = require("node:test");
const assert = require("node:assert/strict");
const { chargerApp, trouver } = require("./dom_shim");

const copie = (x) => JSON.parse(JSON.stringify(x));

/** Texte rendu par un sous-arbre, enfants compris. */
function texteDe(n) {
  if (typeof n === "string") return n;
  if (typeof n !== "object" || n === null) return "";
  return (n.children ?? []).map(texteDe).join("");
}

/** `DetailRun` tel que le backend le sérialise. */
function detail(num, date, jjs, chiffres = {}) {
  const { vise = 0, report = 0, stock = 999, place = vise, reliquat = 0 } = chiffres;
  return {
    run_num: num, run_date: date, jjs, mep_id: 1, mep_date: "2026-08-03",
    vise, report_entrant: report, stock, place, reliquat,
  };
}

/** `PlanApercu` minimal : ce que lit le panneau, rien de plus. */
function apercu(runs) {
  return {
    funnel: { lignes: 100, cf_distincts: 100, jj_valide: 100, resolus: 100,
              ctc_ready: 100, ppf_usable: 100, eligibles: 100 },
    timeline: runs.map((r) => ({
      date: r.date, jour_semaine: "mar", weekend: false, ferie: null, jalons: [],
      runs: [{
        num: r.num, jjs: r.jjs, exclu: !!r.exclu,
        ecart: r.ecart ?? null,
        detail: r.ecart ? null : detail(r.num, r.date, r.jjs, r),
      }],
    })),
    stock_jj: [{ jj: 1, comptes: 700, couvert: true },
               { jj: 5, comptes: 300, couvert: true },
               { jj: 9, comptes: 240, couvert: false }],
    plateformes: [], avertissements: [], meps: ["2026-08-03"],
    cible: 1000, total: 1000, geles: 0, epingles: 0, retires: 0,
  };
}

const TROIS_RUNS = [
  { num: "3320", date: "2026-08-11", jjs: [1, 5], vise: 40 },
  { num: "3327", date: "2026-09-10", jjs: [1, 5], vise: 80 },
  { num: "3331", date: "2026-09-25", jjs: [10, 15], vise: 120 },
];

/** Panneau rendu, aperçu injecté, fenêtre FUT saisie. */
function ecran(runs = TROIS_RUNS) {
  const ctx = chargerApp();
  ctx.evaluer("plan").apercu = ctx.evaluer(`(${JSON.stringify(apercu(runs))})`);
  ctx.app.renderPlanAside();
  ctx.$("plan-debut").value = "2026-08-01";
  ctx.$("plan-fin").value = "2026-11-30";
  return ctx;
}

/** Choisit une forme de rampe comme le ferait l'utilisateur. */
function choisirForme($, forme) {
  const sel = $("plan-forme");
  sel.value = forme;
  sel.listeners.change();
}

// ---------------------------------------------------------------- tâche 2

test("le bloc des volumes n'existe qu'en forme manuelle", () => {
  const { app, $ } = ecran();
  assert.equal(trouver($("plan-aside"), (n) => n.attrs.id === "plan-volumes"), null,
    "forme plate : aucun champ de volume");

  choisirForme($, "manuelle");
  assert.notEqual(trouver($("plan-aside"), (n) => n.attrs.id === "plan-volumes"), null);
  assert.equal(app.planParams().rampe.forme, "manuelle");
});

test("une ligne de saisie par run retenu, aucune pour un run écarté", () => {
  const { $ } = ecran([
    ...TROIS_RUNS,
    { num: "3342", date: "2026-10-24", jjs: [20], ecart: "exclu", exclu: true },
  ]);
  choisirForme($, "manuelle");

  for (const r of TROIS_RUNS) {
    assert.notEqual(trouver($("plan-aside"), (n) => n.attrs.id === `plan-vol-${r.num}`), null,
      `le run retenu ${r.num} doit avoir un champ`);
  }
  assert.equal(trouver($("plan-aside"), (n) => n.attrs.id === "plan-vol-3342"), null,
    "un run écarté n'a pas de volume à saisir");
});

test("un volume saisi survit à la reconstruction du panneau", () => {
  // Les champs sont dynamiques : les laisser au DOM les exposerait au bug
  // corrigé le 26/07, où ajouter une MEP rendait la saisie à ses défauts.
  const { app, $ } = ecran();
  choisirForme($, "manuelle");

  const champ = $("plan-vol-3327");
  champ.value = "175";
  champ.listeners.input({ target: champ });

  $("plan-mepadd").listeners.change({ target: { value: "2026-09-01" } });

  assert.equal($("plan-vol-3327").value, "175");
  assert.equal(copie(app.planParams().rampe.volumes)["3327"], 175);
});

// ---------------------------------------------------------------- tâche 3

test("basculer en manuel part des volumes déjà affichés", () => {
  // Le geste réel est « je prends ma rampe linéaire et j'ajuste deux runs ».
  // Repartir de zéro obligerait à tout ressaisir.
  const { app, $ } = ecran();
  choisirForme($, "manuelle");

  assert.deepEqual(copie(app.planParams().rampe.volumes),
    { 3320: 40, 3327: 80, 3331: 120 });
});

test("revenir au manuel ne réécrase pas une saisie existante", () => {
  const { app, $ } = ecran();
  choisirForme($, "manuelle");
  const champ = $("plan-vol-3320");
  champ.value = "5";
  champ.listeners.input({ target: champ });

  choisirForme($, "lineaire");
  choisirForme($, "manuelle");

  assert.equal(copie(app.planParams().rampe.volumes)["3320"], 5,
    "la saisie de l'utilisateur prime sur le recalcul du moteur");
});

test("en manuel, le pilote n'est ni affiché ni envoyé", () => {
  // Il n'a aucun effet dans cette forme : l'envoyer ferait porter à
  // l'utilisateur un réglage qui n'agit pas, et l'avertissement
  // « cible trop basse pour tenir N par run » désignerait une fausse cause.
  const { app, $ } = ecran();
  const cocher = $("plan-pilote");
  cocher.checked = true;
  cocher.listeners.change();
  assert.notEqual(app.planParams().rampe.pilote, null, "pilote actif en linéaire");

  choisirForme($, "manuelle");

  assert.equal(trouver($("plan-aside"), (n) => n.attrs.id === "plan-pilote"), null,
    "la case pilote quitte l'écran");
  assert.equal(app.planParams().rampe.pilote, null,
    "masquer ne suffit pas : la case garde son état, c'est planParams qui tranche");
});

// ---------------------------------------------------------------- tâche 4

test("un run qui n'absorbe pas son volume porte l'alerte, pas les autres", () => {
  // L'alerte se lit sur `reliquat`, pas sur « volume > stock » : le reliquat
  // tient compte du report entrant, ce qu'un calcul local raterait. Ce n'est
  // pas une erreur — le surplus part sur le run suivant.
  const { $ } = ecran([
    { num: "3336", date: "2026-10-09", jjs: [10], vise: 200, stock: 143, place: 143, reliquat: 57 },
    { num: "3341", date: "2026-10-23", jjs: [20], vise: 260, stock: 402, place: 260, reliquat: 0 },
  ]);
  choisirForme($, "manuelle");

  const alertes = [];
  (function collecte(n) {
    if (typeof n !== "object" || n === null) return;
    if (n.className === "vol over") alertes.push(texteDe(n));
    (n.children ?? []).forEach(collecte);
  })($("plan-aside"));

  assert.equal(alertes.length, 1, "seul le run en dépassement doit alerter");
  assert.match(alertes[0], /3336/);
  assert.match(alertes[0], /stock 143/);
  assert.match(alertes[0], /57 reportés/);
});

// ---------------------------------------------------------------- tâche 5

/** Barres de l'aperçu des volumes, dans l'ordre du rendu. */
function barres($) {
  const box = trouver($("plan-param"), (n) => n.attrs.id === "plan-vol-bars");
  return box ? box.children : [];
}

test("une barre par run retenu, dans l'ordre chronologique", () => {
  // Vaut pour les quatre formes, pas seulement la manuelle.
  const { app, $ } = ecran();
  app.renderPlanParam();

  const b = barres($);
  assert.equal(b.length, 3);
  assert.deepEqual(b.map(texteDe), ["40", "80", "120"]);
});

test("la barre du plus gros volume occupe toute la hauteur", () => {
  const { app, $ } = ecran();
  app.renderPlanParam();

  const hauteurs = barres($).map((n) =>
    trouver(n, (x) => x.tagName === "i").attrs.style);
  assert.match(hauteurs[2], /height:100(\.0)?%/, "le plus gros visé donne l'échelle");
  assert.match(hauteurs[0], /height:33/, "40 sur 120 → un tiers");
});

test("une barre en dépassement se distingue", () => {
  const { app, $ } = ecran([
    { num: "3336", date: "2026-10-09", jjs: [10], vise: 200, stock: 143, place: 143, reliquat: 57 },
    { num: "3341", date: "2026-10-23", jjs: [20], vise: 260, stock: 402, place: 260, reliquat: 0 },
  ]);
  app.renderPlanParam();

  const classes = barres($).map((n) => n.className);
  assert.equal(classes[0], "vol-bar over");
  assert.equal(classes[1], "vol-bar");
});

test("aucun run retenu : pas de graphe du tout, pas un cadre vide", () => {
  // Compter les barres ne suffit pas : un conteneur rendu sans run donnerait
  // zéro barre tout en affichant le titre et sa légende au-dessus du vide.
  const { app, $ } = ecran([
    { num: "3342", date: "2026-10-24", jjs: [20], ecart: "aucune_mep" },
  ]);
  app.renderPlanParam();

  assert.equal(trouver($("plan-param"), (n) => n.attrs.id === "plan-vol-bars"), null,
    "le conteneur lui-même ne doit pas être rendu");
  assert.doesNotMatch(texteDe($("plan-param")), /Volumes par run/,
    "ni son titre");
});

// ---------------------------------------------------------------- tâche 6

/** Charge l'app avec un plan enregistré en base, puis ouvre l'écran. */
async function rouvrirAvec(rampe) {
  const ctx = chargerApp();
  const params = {
    runs: [], debut: "2026-08-01", fin: "2026-11-30", meps: ["2026-08-03"],
    mep_count: 0, cible: 900, seed: 7, pa_exclues: [], rampe,
  };
  ctx.evaluer("plan").apercu = ctx.evaluer(`(${JSON.stringify(apercu(TROIS_RUNS))})`);
  // `plan_load` est la seule commande dont la réponse compte ici.
  ctx.repondreAux((cmd) =>
    cmd === "plan_load" ? { params, fichier: "clients.csv", autre_fichier: false } : null);
  await ctx.app.ouvrirPlan();
  return ctx;
}

test("un plan enregistré en manuel rouvre avec ses volumes", () => {
  // La rampe part déjà en YAML dans `params_yaml` : ce qui manquait, c'est la
  // relecture. `ouvrirPlan` ne restaurait AUCUN paramètre de rampe.
  return rouvrirAvec({ forme: "manuelle", pilote: null, volumes: { 3320: 11, 3327: 22, 3331: 33 } })
    .then(({ app, $ }) => {
      assert.equal($("plan-forme").value, "manuelle");
      assert.equal($("plan-vol-3327").value, "22");
      assert.deepEqual(copie(app.planParams().rampe.volumes),
        { 3320: 11, 3327: 22, 3331: 33 });
    });
});

test("un plan enregistré en géométrique rouvre avec sa raison", () => {
  return rouvrirAvec({ forme: "geometrique", pilote: null, raison: 1.8 })
    .then(({ app, $ }) => {
      assert.equal($("plan-forme").value, "geometrique");
      assert.equal($("plan-raison").value, "1.8");
      assert.equal(app.planParams().rampe.raison, 1.8);
    });
});

test("un plan enregistré avec pilote rouvre la case cochée", () => {
  return rouvrirAvec({ forme: "plate", pilote: { runs: 2, cf_par_run: 15 } })
    .then(({ app, $ }) => {
      assert.equal($("plan-pilote").checked, true);
      assert.deepEqual(copie(app.planParams().rampe.pilote), { runs: 2, cf_par_run: 15 });
    });
});

test("la note sur la cible n'apparaît qu'en manuel", () => {
  // La cible n'est pas neutralisée : `allouer` s'en sert toujours pour les
  // quotas par plateforme. La griser mentirait, il faut donc le dire.
  const { $ } = ecran();
  const note = () => trouver($("plan-aside"), (n) => n.attrs.id === "plan-cible-manuel");
  assert.equal(note(), null);

  choisirForme($, "manuelle");
  assert.match(texteDe(note()), /quotas par plateforme/);
});

// ------------------------------------------- le panneau suit les aperçus

/** Laisse passer l'anti-rebond de `planRecalc` (250 ms) et l'aperçu qui suit. */
const attendreApercu = () => new Promise((r) => setTimeout(r, 320));

test("les champs de volumes apparaissent dès l'arrivée du premier aperçu", async () => {
  // Vécu en application : un plan enregistré en manuel rouvrait avec la bonne
  // forme mais SANS ses champs — le panneau est rendu avant que l'aperçu
  // existe, et rien ne le rerendait ensuite. Il fallait changer de forme puis
  // revenir pour les voir.
  const ctx = chargerApp();
  const params = {
    runs: [{ num: "3320", date: "2026-08-11", jjs: [1], exclu: false }],
    debut: "2026-08-01", fin: "2026-11-30", meps: ["2026-08-03"], mep_count: 0,
    cible: 900, seed: 7, pa_exclues: [],
    rampe: { forme: "manuelle", pilote: null, volumes: { 3320: 40, 3327: 80, 3331: 120 } },
  };
  ctx.repondreAux((cmd) => {
    if (cmd === "plan_load") return { params, fichier: "brm2607.csv", autre_fichier: false };
    if (cmd === "plan_preview") return ctx.evaluer(`(${JSON.stringify(apercu(TROIS_RUNS))})`);
    // `plan_lignes` rend un Vec, jamais null : une doublure qui répond null
    // ferait porter au code un état que le moteur ne produit pas.
    return ctx.evaluer("[]");
  });

  await ctx.app.ouvrirPlan();
  assert.equal(ctx.$("plan-forme").value, "manuelle");

  await attendreApercu();

  assert.equal(ctx.$("plan-vol-3327").value, "80",
    "les champs doivent être là sans qu'on ait à changer de forme");
});

test("le premier aperçu part avec les volumes restaurés", async () => {
  // Vécu en application : les champs du panneau étaient bons, mais la timeline
  // et les barres affichaient zéro partout jusqu'à ce qu'on change de forme.
  // `planParams` lisait les volumes dans l'aperçu — qui n'existe pas encore au
  // premier recalcul, donc le moteur recevait une map vide et rendait un plan
  // vide, cohérent avec ce qu'on lui avait demandé.
  const ctx = chargerApp();
  const params = {
    runs: [{ num: "3320", date: "2026-08-11", jjs: [1], exclu: false }],
    debut: "2026-08-01", fin: "2026-11-30", meps: ["2026-08-03"], mep_count: 0,
    cible: 900, seed: 7, pa_exclues: [],
    rampe: { forme: "manuelle", pilote: null, volumes: { 3320: 40, 3327: 80, 3331: 120 } },
  };
  ctx.repondreAux((cmd) => {
    if (cmd === "plan_load") return { params, fichier: "brm2607.csv", autre_fichier: false };
    if (cmd === "plan_preview") return ctx.evaluer(`(${JSON.stringify(apercu(TROIS_RUNS))})`);
    // `plan_lignes` rend un Vec, jamais null : une doublure qui répond null
    // ferait porter au code un état que le moteur ne produit pas.
    return ctx.evaluer("[]");
  });

  await ctx.app.ouvrirPlan();
  await attendreApercu();

  const appel = ctx.invocations.find(([c]) => c === "plan_preview");
  assert.notEqual(appel, undefined, "un aperçu doit être demandé à l'ouverture");
  assert.deepEqual(copie(appel[1].params.rampe.volumes),
    { 3320: 40, 3327: 80, 3331: 120 },
    "le tout premier aperçu doit déjà porter les volumes du plan enregistré");
});

test("une saisie en cours n'est pas interrompue par un aperçu", async () => {
  // Le panneau suit les aperçus, mais le reconstruire pendant une frappe ferait
  // perdre le focus au champ — et c'est la frappe elle-même qui déclenche le
  // recalcul.
  const ctx = ecran();
  // `planRecalc` sort par son retour anticipé sans calendrier chargé : sans
  // ces runs, le test n'atteindrait jamais le code qu'il prétend couvrir.
  ctx.evaluer("plan").runs = ctx.evaluer(`(${JSON.stringify(
    TROIS_RUNS.map((r) => ({ num: r.num, date: r.date, jjs: r.jjs, exclu: false })))})`);
  ctx.repondreAux(() => ctx.evaluer(`(${JSON.stringify(apercu(TROIS_RUNS))})`));
  choisirForme(ctx.$, "manuelle");

  const champ = ctx.$("plan-vol-3327");
  champ.focus();
  champ.value = "175";
  champ.listeners.input({ target: champ });

  await attendreApercu();

  assert.equal(ctx.$("plan-vol-3327"), champ, "le champ ne doit pas avoir été remplacé");
  assert.equal(ctx.$("plan-vol-3327").value, "175");
});

test("« Tout à 0 » remet chaque run à zéro", () => {
  const { app, $ } = ecran();
  choisirForme($, "manuelle");
  assert.equal($("plan-vol-3320").value, "40", "on part bien de volumes non nuls");

  trouver($("plan-aside"), (n) => n.attrs.id === "plan-vol-zero").listeners.click();

  // Une map vide et une map de zéros sont équivalentes pour le moteur — un run
  // absent vaut 0 (`rampe_manuelle_rend_les_volumes_verbatim`). Ce qui doit
  // être vrai, c'est qu'aucun volume ne subsiste et que les runs restent
  // listés, à zéro, prêts à être resaisis.
  assert.deepEqual(copie(app.planParams().rampe.volumes), {});
  assert.equal($("plan-vol-3320").value, "0");
  assert.equal($("plan-vol-3331").value, "0");
});
