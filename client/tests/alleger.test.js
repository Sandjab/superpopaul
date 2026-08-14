// Modale « Alléger un run » : trois gestes, un seul retrait.
//
// Le calcul de la répartition est côté Rust (`proposer_retrait_proportionnel`,
// déjà testé) : ce qui est vérifié ici, c'est le câblage — QUELS comptes
// partent au retrait selon le mode, et le fait qu'il n'en parte qu'UN SEUL
// appel. C'est ce dernier point qui fait « un geste » aux yeux du rapport de
// rapprochement : un retrait par compte redonnerait 143 fois la même phrase.

const test = require("node:test");
const assert = require("node:assert/strict");
const { chargerApp, trouver } = require("./dom_shim");

/** Une ligne du plan telle que `plan_lignes` la rend (`LigneRecap`). */
function ligne(cf, extra = {}) {
  return {
    cf, participant: `iso6523-actorid-upis::0225:${cf}`, raison_sociale: `Société ${cf}`,
    jj: 5, pa: "Cegedim", mep_id: 1, mep_date: "2026-09-01", run_num: "RF01",
    run_date: "2026-09-10", origine: "auto", etat: "eligible", gelee: false,
    retire_motif: null, ...extra,
  };
}

/** Écran du plan avec `lignes` déjà chargées, et un backend qui répond au
 *  strict nécessaire de l'épilogue (rechargement du récap, retrait). */
function ecran(lignes, repondre = () => null) {
  const ctx = chargerApp();
  const p = ctx.evaluer("plan");
  p.genere = true;
  p.lignes = ctx.evaluer(`(${JSON.stringify(lignes)})`);
  ctx.repondreAux((cmd, args) => {
    if (cmd === "plan_lignes") return ctx.evaluer(`(${JSON.stringify(lignes)})`);
    if (cmd === "plan_retirer") return ctx.evaluer("([])");
    return repondre(cmd, args);
  });
  return ctx;
}

/** Premier bouton de la modale dont le libellé commence par `debut`. */
const bouton = ($, debut) => trouver($("modal"),
  (n) => n.tagName === "button" && n.textContent.startsWith(debut));

const champMotif = ($) => trouver($("modal"), (n) => n.tagName === "textarea");

/** Saisit un motif comme l'utilisateur : la valeur ET la notification, sans
 *  laquelle le bouton de retrait ne rouvre jamais. */
function taperMotif($, texte) {
  const zone = champMotif($);
  zone.value = texte;
  zone.listeners.input({ target: zone });
  return zone;
}

/** Toutes les cases à cocher de la modale, en profondeur d'abord. */
function casesACocher(noeud, out = []) {
  if (typeof noeud !== "object" || noeud === null) return out;
  if (noeud.attrs?.type === "checkbox") out.push(noeud);
  for (const enfant of noeud.children ?? []) casesACocher(enfant, out);
  return out;
}

function cocher(cb) {
  cb.checked = true;
  cb.listeners.change({ target: cb });
}

/** Les appels `plan_retirer` partis vers le backend, comptes triés. */
const retraits = (ctx) => ctx.invocations
  .filter(([c]) => c === "plan_retirer")
  .map(([, a]) => ({ cfs: [...a.cfs].sort(), motif: a.motif }));

// Un run passé l'est pour toujours ; un run « à venir » daté 2099 le reste
// aussi longtemps que ces tests vivront. Les deux dates sont choisies pour que
// l'horloge de la machine ne décide de rien.
const JOUR_PASSE = { date: "2026-01-05" };
const JOUR_FUTUR = { date: "2099-01-05" };
const RUN = { num: "RF01", jjs: [5] };

/** Un run de la timeline, tel que `PlanApercu` le rend. */
const runTimeline = (extra = {}) => ({
  num: "RF01", jjs: [5], exclu: false, ecart: null,
  detail: { vise: 40, report_entrant: 0, stock: 999, place: 40, reliquat: 0 },
  ...extra,
});

/** Les libellés des boutons d'action de la cellule `tl-add` d'une ligne de run. */
function actionsDuRun(ctx, run, jour = JOUR_FUTUR) {
  const tr = ctx.app.ligneRun(ctx.evaluer(`(${JSON.stringify(jour)})`),
    ctx.evaluer(`(${JSON.stringify(run)})`));
  const cellule = trouver(tr, (n) => n.className === "tl-add");
  const libelles = [];
  (function marcher(n) {
    if (typeof n !== "object" || n === null) return;
    if (n.tagName === "button") libelles.push(n.textContent);
    for (const c of n.children ?? []) marcher(c);
  })(cellule);
  return libelles;
}

test("l'action d'allégement vit sur la ligne du run, à côté de l'ajout", () => {
  // « Alléger » est une décision sur UN run — elle se prend là où le run se
  // lit, pas sur une sélection du récap. Les deux gestes exigent un plan
  // ENREGISTRÉ : `plan_retirer` retouche le plan persisté.
  const ctx = ecran([ligne("CF1")]);
  assert.deepEqual(actionsDuRun(ctx, runTimeline()), ["+ Ajouter", "Alléger…"]);

  ctx.evaluer("plan").genere = false;
  assert.deepEqual(actionsDuRun(ctx, runTimeline()), [],
    "sans plan enregistré, les deux gestes n'ont rien à retoucher");
});

test("un run écarté ne porte aucune action, allégement compris", () => {
  // On ne peut rien placer sur un run écarté, ni rien lui retirer : il n'a
  // jamais reçu de compte.
  const ctx = ecran([ligne("CF1")]);
  assert.deepEqual(actionsDuRun(ctx, runTimeline({ ecart: "exclu", detail: null })), []);
});

test("le déclencheur ouvre la modale sur SON run et SON jour porteur", async () => {
  // `RunJour` ne porte pas de date : elle vient du jour civil qui l'héberge.
  // Passer le run sans son jour donnerait un motif pré-rempli sans date, et
  // surtout un run à venir traité comme passé.
  const ctx = ecran([ligne("CF1"), ligne("CF2", { run_num: "RF09" })]);
  const tr = ctx.app.ligneRun(ctx.evaluer('({ date: "2026-01-05" })'),
    ctx.evaluer(`(${JSON.stringify(runTimeline())})`));
  const declencheur = trouver(tr, (n) => n.tagName === "button" && n.textContent === "Alléger…");

  await declencheur.click();

  const texte = ctx.$("modal").textContent;
  assert.match(texte, /Alléger le run RF01/, "la modale doit porter le run cliqué");
  assert.match(champMotif(ctx.$).value, /du 05\/01\/2026 exclu a posteriori/,
    "la date du jour porteur voyage avec le run, et le passé impose l'exclusion");
  assert.equal(bouton(ctx.$, "Retirer 1 compte(s)").disabled, true,
    "un seul compte actif sur RF01 : CF2 est sur un autre run");
});

test("run passé : seul l'exclusion est offerte, et le motif reste à compléter", async () => {
  // Le run a déjà été joué : le rééquilibrer n'a pas de sens, il s'exclut en
  // entier — épinglées comprises. Et le motif pré-rempli dit QUEL run est
  // exclu, jamais POURQUOI : c'est la cause que le rapport transmettra.
  const ctx = ecran([
    ligne("CF1"), ligne("CF2"), ligne("CF3"),
    ligne("CF4", { retire_motif: "retiré la semaine dernière" }),
  ]);
  ctx.app.ouvrirAllegerRun(RUN, JOUR_PASSE);

  assert.equal(bouton(ctx.$, "Ne garder que"), null,
    "un run passé n'offre pas la sélection : il est déjà joué");
  const retirer = bouton(ctx.$, "Retirer 3 compte(s)");
  assert.ok(retirer, "le geste porte sur les 3 lignes ACTIVES du run, pas sur les 4 lignes");
  assert.equal(retirer.disabled, true, "le motif pré-rempli ne dit aucune cause");

  const zone = champMotif(ctx.$);
  assert.match(zone.value, /Run RF01 du 05\/01\/2026 exclu a posteriori/,
    "le motif doit nommer le run et sa date, en clair");

  taperMotif(ctx.$, `${zone.value}incident chez le prestataire`);
  assert.equal(bouton(ctx.$, "Retirer 3 compte(s)").disabled, false,
    "une cause écrite après le tiret ouvre le geste");

  await bouton(ctx.$, "Retirer 3 compte(s)").click();

  const partis = retraits(ctx);
  assert.equal(partis.length, 1, "un geste = un seul retrait, c'est ce que le rapport regroupe");
  assert.deepEqual(partis[0].cfs, ["CF1", "CF2", "CF3"]);
  assert.match(partis[0].motif, /incident chez le prestataire$/);
});

test("run à venir, mode sélection : ce qui n'est pas coché part au retrait", async () => {
  const ctx = ecran([ligne("CF1"), ligne("CF2"), ligne("CF3")]);
  ctx.app.ouvrirAllegerRun(RUN, JOUR_FUTUR);

  bouton(ctx.$, "Ne garder que ma sélection").click();
  const cases = casesACocher(ctx.$("modal"));
  assert.equal(cases.length, 3, "une case par compte actif du run, aucune case « tout »");
  assert.ok(cases.every((c) => !c.checked), "on part de rien gardé, comme la maquette l'annonce");
  assert.match(ctx.$("modal").textContent, /0 gardé\(s\) — 3 seront retiré\(s\)/,
    "le pied compte les deux côtés dès l'ouverture");

  cocher(cases[0]);
  assert.match(ctx.$("modal").textContent, /1 gardé\(s\) — 2 seront retiré\(s\)/,
    "le pied suit chaque clic : c'est lui qui dit ce que le geste va faire");

  taperMotif(ctx.$, "seul le compte pilote reste sur ce run");
  await bouton(ctx.$, "Retirer 2 compte(s)").click();

  const partis = retraits(ctx);
  assert.equal(partis.length, 1);
  assert.deepEqual(partis[0].cfs, ["CF2", "CF3"], "on coche ce qu'on GARDE, le reste part");
});

/** Proposition du backend : 1 Cegedim sur 2 actifs, 1 Esalink sur 1 actif. */
const PROPOSITION = [
  { pa: "Cegedim", retirer: ["CF3"], actifs: 2 },
  { pa: "Esalink", retirer: ["CF2"], actifs: 1 },
];

/** Run à venir de 3 comptes, deux plateformes, prêt pour le mode prorata. */
function ecranProrata() {
  const ctx = ecran(
    [ligne("CF1"), ligne("CF2", { pa: "Esalink" }), ligne("CF3")],
    (cmd) => (cmd === "plan_proposer_retrait"
      ? ctx.evaluer(`(${JSON.stringify(PROPOSITION)})`) : null));
  ctx.app.ouvrirAllegerRun(RUN, JOUR_FUTUR);
  return ctx;
}

test("run à venir, mode prorata : la proposition part au retrait telle quelle", async () => {
  // Aucune répartition n'est calculée ici : l'IHM demande N, affiche ce que le
  // backend a choisi, et retire exactement cela.
  const ctx = ecranProrata();
  const champN = trouver(ctx.$("modal"), (n) => n.attrs?.type === "number");
  assert.ok(champN, "le mode prorata s'ouvre par défaut sur un run à venir");
  champN.value = "2";
  await bouton(ctx.$, "Proposer").click();

  const demande = ctx.invocations.find(([c]) => c === "plan_proposer_retrait");
  assert.ok(demande, "le calcul doit partir au backend, jamais être refait en JS");
  assert.deepEqual({ ...demande[1] }, { runNum: "RF01", n: 2 });
  assert.match(ctx.$("modal").textContent, /Cegedim.*1 sur 2/s,
    "la proposition se lit par plateforme : c'est ce qui prouve la répartition conservée");

  taperMotif(ctx.$, "volume du run revu à la baisse");
  await bouton(ctx.$, "Retirer 2 compte(s)").click();

  const partis = retraits(ctx);
  assert.equal(partis.length, 1);
  assert.deepEqual(partis[0].cfs, ["CF2", "CF3"]);
});

test("un compte proposé s'échange contre un actif de la MÊME plateforme", async () => {
  // L'échange est ce qui rend la proposition amendable sans casser ce qu'elle
  // garantit : le nombre de retirés par plateforme ne bouge pas.
  const ctx = ecranProrata();
  trouver(ctx.$("modal"), (n) => n.attrs?.type === "number").value = "2";
  await bouton(ctx.$, "Proposer").click();

  bouton(ctx.$, "échanger").click(); // le premier proposé : CF3, Cegedim
  assert.equal(trouver(ctx.$("modal"),
    (n) => n.className === "cand" && n.textContent.includes("CF2")), null,
    "CF2 est sur une autre plateforme : l'échanger casserait la répartition");
  const candidat = trouver(ctx.$("modal"),
    (n) => n.className === "cand" && n.textContent.includes("CF1"));
  assert.ok(candidat, "CF1 est le seul autre compte Cegedim actif du run");

  trouver(candidat, (n) => n.tagName === "button").click();

  taperMotif(ctx.$, "arbitrage du comité");
  await bouton(ctx.$, "Retirer 2 compte(s)").click();

  assert.deepEqual(retraits(ctx)[0].cfs, ["CF1", "CF2"],
    "l'échange remplace le compte dans la proposition, sans en changer le nombre");
});

test("une erreur du backend s'affiche dans la modale, qui reste ouverte", async () => {
  // « le maximum est 4 » est une information utilisable : elle doit rester
  // sous les yeux, à côté du champ qu'elle invite à corriger.
  const MESSAGE = "impossible de retirer 9 compte(s) : le maximum est 4 <script>alert(1)</script>";
  const ctx = ecran([ligne("CF1"), ligne("CF2")], (cmd) => {
    if (cmd === "plan_proposer_retrait") throw MESSAGE;
    return null;
  });
  ctx.app.ouvrirAllegerRun(RUN, JOUR_FUTUR);
  trouver(ctx.$("modal"), (n) => n.attrs?.type === "number").value = "9";
  await bouton(ctx.$, "Proposer").click();

  assert.ok(ctx.$("modal").textContent.includes("le maximum est 4"),
    "le message du backend doit être lisible sans fermer la fenêtre");
  assert.equal(trouver(ctx.$("modal"), (n) => n.tagName === "script"), null,
    "un message d'erreur reste une donnée : jamais d'innerHTML");
  assert.equal(ctx.$("modal-backdrop").className.includes("hidden"), false,
    "la modale reste ouverte : le champ à corriger est dedans");
});

test("l'avertissement de MEP gelée porte sur les comptes qui vont être retirés", async () => {
  // Retirer d'une MEP livrée change un fichier déjà transmis. Le compte gelé
  // n'est ici PAS gardé : c'est bien lui qui part, l'avertissement doit sortir.
  const ctx = ecran([
    ligne("CF1", { gelee: true, mep_date: "2026-05-15" }),
    ligne("CF2"),
  ]);
  ctx.app.ouvrirAllegerRun(RUN, JOUR_FUTUR);
  bouton(ctx.$, "Ne garder que ma sélection").click();

  const alerte = () => trouver(ctx.$("modal"), (n) => n.className === "danger-note");
  assert.ok(alerte(), "rien n'est gardé : le compte gelé part, il faut le dire");
  assert.match(alerte().textContent, /15\/05\/2026/, "la MEP concernée est nommée, en clair");

  cocher(casesACocher(ctx.$("modal"))[0]); // on garde le gelé
  assert.equal(alerte(), null,
    "le gelé est gardé : l'avertissement doit tomber, sinon il crie sans raison");
});
