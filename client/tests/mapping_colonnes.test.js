// Mapping des colonnes retrouvé par signature d'en-têtes.
//
// Vécu en application : après une relance, le mapping du plan est vide (il
// appartient au profil, pas aux réglages). Rechargé « le même fichier », les
// colonnes étaient à re-désigner de mémoire — et une erreur de désignation
// vidait l'écran en silence. La signature des en-têtes rend le mapping
// retrouvable sans dépendre du chemin, qui bouge et se duplique.

const test = require("node:test");
const assert = require("node:assert/strict");
const { chargerApp } = require("./dom_shim");

const ENTETES = ["CF_ID", "ADRESSAGE_ID", "RAISON_SOCIALE", "ACTG_CYCLE_DOM"];

function preview(hash, headers = ENTETES) {
  return {
    headers, rows: [headers.map((_, i) => `v${i}`)],
    delimiter: ";", encoding: "utf-8", columns_hash: hash,
    size_bytes: 1024, suggested_pid_column: 1,
  };
}

/** Réglages tels que `load_settings` les rend. */
function reglages(mappings) {
  return { version: 1, api: {}, output: { timestamp_suffix: true }, ppf: {}, mappings };
}

/** App chargée, `preview_csv` armé sur `p`. */
function app(mappings, p) {
  const ctx = chargerApp();
  ctx.repondreAux((cmd, args) => {
    if (cmd === "load_settings") {
      return { version: 1, api: {}, output: { timestamp_suffix: true }, mappings };
    }
    if (cmd === "preview_csv") return p;
    // La commande rend les réglages à jour ; le test se contente de l'écho.
    if (cmd === "remember_columns") return args.settings;
    return null;
  });
  return ctx;
}

test("un mapping mémorisé est restauré pour la même signature d'en-têtes", async () => {
  const memo = [{
    columns_hash: "abc123", pid_column: "ADRESSAGE_ID", cf_column: "CF_ID",
    jj_column: "ACTG_CYCLE_DOM", raison_sociale_column: "RAISON_SOCIALE",
  }];
  const { app: a, evaluer } = app(memo, preview("abc123"));

  a.applySettings(reglages(memo));
  await a.pickInput("/tmp/brm2607.csv");

  const input = evaluer("state").config.input;
  assert.equal(input.jj_column, "ACTG_CYCLE_DOM", "la colonne du plan doit revenir seule");
  assert.equal(input.cf_column, "CF_ID");
  assert.equal(input.raison_sociale_column, "RAISON_SOCIALE");
  assert.equal(input.pid_column, "ADRESSAGE_ID");
});

test("une signature inconnue laisse le mapping vide", async () => {
  // Structure différente : les noms mémorisés ne désignent plus rien. Restaurer
  // à l'aveugle rejouerait exactement le bug qu'on corrige.
  const memo = [{
    columns_hash: "abc123", pid_column: "ADRESSAGE_ID", cf_column: "CF_ID",
    jj_column: "ACTG_CYCLE_DOM", raison_sociale_column: "",
  }];
  const { app: a, evaluer } = app(memo, preview("zzz999", ["A", "B", "C"]));

  a.applySettings(reglages(memo));
  await a.pickInput("/tmp/autre.csv");

  const input = evaluer("state").config.input;
  assert.equal(input.cf_column, "");
  assert.equal(input.jj_column, "");
});

test("désigner une colonne la mémorise pour cette signature", async () => {
  const { app: a, evaluer, invocations } = app([], preview("abc123"));
  a.applySettings(reglages([]));
  await a.pickInput("/tmp/brm2607.csv");

  evaluer("state").config.input.jj_column = "ACTG_CYCLE_DOM";
  evaluer("state").config.input.cf_column = "CF_ID";
  await a.memoriserColonnes();

  const appel = invocations.findLast(([c]) => c === "remember_columns");
  assert.notEqual(appel, undefined, "la désignation doit être poussée aux réglages");
  assert.equal(appel[1].mapping.columns_hash, "abc123");
  assert.equal(appel[1].mapping.jj_column, "ACTG_CYCLE_DOM");
});

test("désigner depuis le panneau du plan mémorise sans autre geste", async () => {
  // La fonction peut exister et n'être appelée nulle part : c'est le câblage
  // qui casse, pas le calcul.
  const { app: a, $, invocations } = app([], preview("abc123"));
  a.applySettings(reglages([]));
  await a.pickInput("/tmp/brm2607.csv");
  a.renderPlanAside();

  const sel = $("plan-col-jj");
  sel.value = "ACTG_CYCLE_DOM";
  await sel.listeners.change({ target: sel });

  const appel = invocations.findLast(([c]) => c === "remember_columns");
  assert.notEqual(appel, undefined, "un changement de colonne doit être mémorisé");
  assert.equal(appel[1].mapping.jj_column, "ACTG_CYCLE_DOM");
});

test("désigner l'adressage mémorise aussi", async () => {
  const { app: a, invocations } = app([], preview("abc123"));
  a.applySettings(reglages([]));
  await a.pickInput("/tmp/brm2607.csv");

  await a.designatePid("ADRESSAGE_ID");

  const appel = invocations.findLast(([c]) => c === "remember_columns");
  assert.notEqual(appel, undefined);
  assert.equal(appel[1].mapping.pid_column, "ADRESSAGE_ID");
});

test("sans fichier chargé, rien n'est mémorisé", async () => {
  // Pas de signature : mémoriser attacherait le mapping à n'importe quoi.
  const { app: a, invocations, plaintes } = app([], preview("abc123"));
  a.applySettings(reglages([]));

  await a.memoriserColonnes();

  assert.equal(invocations.filter(([c]) => c === "remember_columns").length, 0);
  // La garde doit court-circuiter proprement. Sans cette seconde assertion, le
  // test passerait aussi bien si la garde disparaissait : l'accès à une
  // signature absente lèverait, et le `catch` avalerait — même absence
  // d'invocation, pour une raison qui n'a rien à voir.
  assert.deepEqual(plaintes, [], "aucune erreur ne doit être avalée en chemin");
});
