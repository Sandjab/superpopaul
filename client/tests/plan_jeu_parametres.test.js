// Jeux de paramètres du plan de charge.
//
// Un seul plan est mémorisé, et ses lignes préservées (gelées, épinglées,
// retirées) suivent d'un fichier à l'autre. Ces tests tiennent les deux gestes
// qui répondent à ça : charger un jeu enregistré, et repartir de zéro — plus la
// confirmation, qui ne doit se poser QUE s'il y a quelque chose à perdre.

const test = require("node:test");
const assert = require("node:assert/strict");
const { chargerApp, trouver } = require("./dom_shim");

/** Jeu complet, tel que le rend `plan_params_load`. */
const JEU = {
  runs: [{ num: "3320", date: "2026-08-11", jjs: [1, 5], exclu: false }],
  debut: "2026-08-01", fin: "2026-11-30", meps: ["2026-09-01"],
  mep_count: 2, cible: 500, seed: 7, pa_exclues: [],
  rampe: { forme: "plate", pilote: null },
};

/** Une ligne du récapitulatif — c'est d'elle que se déduisent les décisions. */
function ligne(cf, extra) {
  return {
    cf, participant: "x", raison_sociale: "S", jj: 5, pa: "PA",
    mep_id: 1, mep_date: "2026-09-01", run_num: "3320", run_date: "2026-09-10",
    origine: "auto", etat: "eligible", gelee: false, retire_motif: null, ...extra,
  };
}

/** Écran de plan rendu, avec les lignes de récapitulatif fournies. */
function ecran(lignes = []) {
  const ctx = chargerApp();
  ctx.repondreAux(() => ctx.evaluer("[]"));
  ctx.repondreAuxDialogues(() => "/tmp/fut-2026-t3.yaml");
  ctx.evaluer("plan").lignes = ctx.evaluer(`(${JSON.stringify(lignes)})`);
  ctx.app.renderPlanAside();
  return ctx;
}

const bouton = (ctx, libelle) =>
  trouver(ctx.$("plan-aside"), (n) => n.tagName === "button" && String(n.children?.[0]) === libelle);
const boutonModale = (ctx, libelle) =>
  trouver(ctx.$("modal"), (n) => n.tagName === "button" && String(n.children?.[0]) === libelle);
/** Une fenêtre est ouverte si elle a un contenu ET que son fond est levé. Le
 *  contenu compte : un élément neuf du faux DOM ne porte AUCUNE classe, donc
 *  « le fond n'est pas masqué » y est vrai avant toute ouverture — l'index.html
 *  réel, lui, pose `hidden` dès le départ. */
const modaleOuverte = (ctx) =>
  ctx.$("modal").children.length > 0 && !ctx.$("modal-backdrop").classList.contains("hidden");
const partis = (ctx, cmd) => ctx.invocations.filter(([c]) => c === cmd).length;

/** Texte visible d'un sous-arbre : `h()` empile les chaînes telles quelles, et
 *  un nombre mis en valeur vit dans un <b> séparé de son libellé. */
function texte(noeud) {
  if (typeof noeud === "string") return noeud;
  if (typeof noeud !== "object" || noeud === null) return "";
  return (noeud.children ?? []).map(texte).join("");
}

// ------------------------------------------------------------ enregistrement

test("enregistrer envoie les paramètres du panneau", async () => {
  const ctx = ecran();
  ctx.$("plan-debut").value = "2026-08-01";
  ctx.$("plan-fin").value = "2026-11-30";
  ctx.$("plan-seed").value = "7";

  await bouton(ctx, "Enregistrer…").click();

  const [, args] = ctx.invocations.find(([c]) => c === "plan_params_save") ?? [];
  assert.ok(args, "aucun enregistrement n'est parti");
  assert.equal(args.params.debut, "2026-08-01");
  assert.equal(args.params.seed, 7, "le seed départage le tirage : le perdre change le plan");
});

// ------------------------------------------------------------ chargement

test("charger un jeu remplit le panneau", async () => {
  const ctx = ecran();
  ctx.repondreAux((cmd) => (cmd === "plan_params_load" ? ctx.evaluer(`(${JSON.stringify(JEU)})`)
                                                       : ctx.evaluer("[]")));

  await bouton(ctx, "Charger…").click();

  assert.equal(ctx.$("plan-debut").value, "2026-08-01");
  assert.equal(ctx.$("plan-cible").value, "500");
  assert.equal(ctx.$("plan-seed").value, "7");
  assert.equal(ctx.evaluer("plan").runs.length, 1, "le calendrier fait partie du jeu");
});

test("un jeu illisible ne modifie rien", async () => {
  // Refus sec : un jeu à moitié appliqué produirait un plan que personne ne
  // saurait expliquer.
  const ctx = ecran();
  ctx.$("plan-debut").value = "2026-01-01";
  ctx.repondreAux((cmd) => {
    if (cmd === "plan_params_load") throw new Error("paramètres illisibles : missing field `seed`");
    return ctx.evaluer("[]");
  });

  await bouton(ctx, "Charger…").click();

  assert.equal(ctx.$("plan-debut").value, "2026-01-01", "le panneau doit rester intact");
  assert.equal(ctx.$("plan-banner").className, "error", "et l'échec doit être dit");
});

// ------------------------------------------------------------ confirmation

test("sans décision au plan, charger ne demande rien", async () => {
  const ctx = ecran([ligne("CF1"), ligne("CF2")]);
  ctx.repondreAux((cmd) => (cmd === "plan_params_load" ? ctx.evaluer(`(${JSON.stringify(JEU)})`)
                                                       : ctx.evaluer("[]")));

  await bouton(ctx, "Charger…").click();

  assert.equal(modaleOuverte(ctx), false, "un plan sans décision n'a rien à perdre");
  assert.equal(ctx.$("plan-debut").value, "2026-08-01", "le jeu doit être appliqué directement");
});

test("les décisions du plan sont comptées sans se recouper", async () => {
  // Ordre de `Preserves::depuis` : retirée l'emporte sur gelée, gelée sur
  // épinglée. Compter autrement donnerait un total supérieur au nombre de
  // lignes — et ferait mentir la fenêtre au moment d'un geste destructeur.
  const ctx = ecran([
    ligne("CF1", { gelee: true }),
    ligne("CF2", { origine: "manuel" }),
    ligne("CF3", { retire_motif: "clôturé", gelee: true, origine: "manuel" }),
    ligne("CF4"),
  ]);
  ctx.repondreAux((cmd) => (cmd === "plan_params_load" ? ctx.evaluer(`(${JSON.stringify(JEU)})`)
                                                       : ctx.evaluer("[]")));

  await bouton(ctx, "Charger…").click();

  assert.equal(modaleOuverte(ctx), true, "il y a des décisions : la question se pose");
  const vu = texte(ctx.$("modal"));
  assert.match(vu, /1 MEP gelée/, `la gelée seule doit compter pour 1 : ${vu}`);
  assert.match(vu, /1 compte épinglé/, `l'épinglée seule doit compter pour 1 : ${vu}`);
  assert.match(vu, /1 compte retiré/, `la retirée compte comme retirée, pas trois fois : ${vu}`);
});

test("« conserver » applique le jeu sans effacer le plan", async () => {
  const ctx = ecran([ligne("CF1", { retire_motif: "clôturé" })]);
  ctx.repondreAux((cmd) => (cmd === "plan_params_load" ? ctx.evaluer(`(${JSON.stringify(JEU)})`)
                                                       : ctx.evaluer("[]")));
  await bouton(ctx, "Charger…").click();

  await boutonModale(ctx, "Conserver ces décisions").click();

  assert.equal(partis(ctx, "plan_reset"), 0, "rien ne doit être effacé");
  assert.equal(ctx.$("plan-debut").value, "2026-08-01", "mais le jeu doit être appliqué");
  assert.equal(modaleOuverte(ctx), false);
});

test("« repartir de zéro » efface le plan et applique le jeu", async () => {
  const ctx = ecran([ligne("CF1", { retire_motif: "clôturé" })]);
  ctx.repondreAux((cmd) => (cmd === "plan_params_load" ? ctx.evaluer(`(${JSON.stringify(JEU)})`)
                                                       : ctx.evaluer("[]")));
  await bouton(ctx, "Charger…").click();

  await boutonModale(ctx, "Repartir de zéro").click();

  assert.equal(partis(ctx, "plan_reset"), 1, "le plan doit être effacé");
  assert.equal(ctx.$("plan-debut").value, "2026-08-01", "et le jeu appliqué");
});

// ------------------------------------------------------------ repartir de zéro

test("« repartir de zéro » n'est proposé que s'il y a un plan", () => {
  // Un bouton destructeur qui ne détruit rien est du bruit.
  assert.equal(bouton(ecran([]), "Repartir de zéro…"), null, "sans plan, pas de bouton");
  assert.ok(bouton(ecran([ligne("CF1")]), "Repartir de zéro…"), "avec un plan, il est là");
});

test("repartir de zéro efface les lignes et garde les paramètres", async () => {
  const ctx = ecran([ligne("CF1", { retire_motif: "clôturé" })]);
  ctx.$("plan-debut").value = "2026-08-01";

  await bouton(ctx, "Repartir de zéro…").click();
  assert.equal(modaleOuverte(ctx), true, "un geste destructeur se confirme");
  await boutonModale(ctx, "Repartir de zéro").click();

  assert.equal(partis(ctx, "plan_reset"), 1);
  assert.equal(ctx.$("plan-debut").value, "2026-08-01",
    "les paramètres du panneau ne sont PAS touchés — c'est tout l'intérêt du geste");
});
