// Réactivation d'un compte retiré.
//
// La fenêtre de retrait promet « Le retrait est annulable » : la commande
// existait, elle n'était branchée nulle part. Ces tests portent sur le câblage
// de l'action et sur ce qu'elle envoie au moteur — un retrait n'est pas une
// suppression, le réactiver doit rester exact.

const test = require("node:test");
const assert = require("node:assert/strict");
const { chargerApp, trouver } = require("./dom_shim");

/** Franchit la frontière de realm : un tableau rendu par le contexte `vm` n'a
 *  pas le prototype `Array` du test, et `deepEqual` strict s'y casse. */
const copie = (x) => JSON.parse(JSON.stringify(x));

function ligne(cf, extra = {}) {
  return {
    cf,
    participant: `iso6523-actorid-upis::0225:${cf}`,
    raison_sociale: `Société ${cf}`,
    jj: 5,
    pa: "Cegedim",
    mep_id: 1,
    mep_date: "2026-09-01",
    run_num: "3320",
    run_date: "2026-09-10",
    origine: "auto",
    etat: "eligible",
    gelee: false,
    retire_motif: null,
    ...extra,
  };
}

const RETIREE = (cf, extra = {}) => ligne(cf, { retire_motif: "incident connu", ...extra });

/** Récapitulatif rendu, avec une sélection posée. */
function recap(lignes, selection) {
  const ctx = chargerApp();
  const p = ctx.evaluer("plan");
  p.lignes = ctx.evaluer(`(${JSON.stringify(lignes)})`);
  p.sel = ctx.evaluer(`(new Set(${JSON.stringify(selection)}))`);
  ctx.app.renderPlanRecap();
  return ctx;
}

/** Bouton de la barre de sélection dont le libellé commence par `debut`. */
function boutonSel($, debut) {
  return trouver($("plan-recap"), (n) =>
    n.tagName === "button" && String(n.children[0] ?? "").startsWith(debut));
}

/** Bouton de la modale dont le libellé commence par `debut`. */
function boutonModale($, debut) {
  return trouver($("modal"), (n) =>
    n.tagName === "button" && String(n.children[0] ?? "").startsWith(debut));
}

test("sans ligne retirée dans la sélection, aucune réactivation n'est proposée", () => {
  // L'action n'a pas de sens sur des lignes actives : la proposer pour la
  // refuser ensuite serait hostile.
  const { $ } = recap([ligne("CF1"), ligne("CF2")], ["CF1", "CF2"]);
  assert.equal(boutonSel($, "Réactiver"), null);
  assert.notEqual(boutonSel($, "Retirer"), null, "les autres actions restent offertes");
});

test("le bouton compte les retirées, pas la sélection", () => {
  // Sélection mixte : cinq comptes, deux retirés. Le libellé annonce ce sur
  // quoi l'action portera vraiment.
  const lignes = [RETIREE("CF1"), ligne("CF2"), RETIREE("CF3"), ligne("CF4"), ligne("CF5")];
  const { $ } = recap(lignes, ["CF1", "CF2", "CF3", "CF4", "CF5"]);
  const b = boutonSel($, "Réactiver");
  assert.notEqual(b, null, "le bouton doit apparaître dès une ligne retirée");
  assert.match(String(b.children[0]), /\b2\b/, `libellé : ${b.children[0]}`);
});

test("seuls les comptes retirés partent au moteur", async () => {
  // LE test de la correction : envoyer un compte actif à `plan_annuler_retrait`
  // ne ferait rien de bon — et sur une sélection mixte, c'est l'erreur facile.
  const lignes = [RETIREE("CF1"), ligne("CF2"), RETIREE("CF3")];
  const ctx = recap(lignes, ["CF1", "CF2", "CF3"]);
  ctx.repondreAux((cmd) => (cmd === "plan_lignes" ? ctx.evaluer("[]") : ctx.evaluer("[]")));

  boutonSel(ctx.$, "Réactiver").listeners.click();
  await boutonModale(ctx.$, "Réactiver").click();

  const appel = ctx.invocations.find(([c]) => c === "plan_annuler_retrait");
  assert.notEqual(appel, undefined, "la commande doit être appelée");
  assert.deepEqual(copie(appel[1].cfs), ["CF1", "CF3"]);
});

test("une MEP gelée est signalée avant de réactiver", async () => {
  // Symétrique du retrait : réactiver sur une MEP livrée change un fichier
  // déjà transmis. Les deux gestes doivent le dire de la même façon.
  const lignes = [RETIREE("CF1", { gelee: true, mep_date: "2026-08-01" }), RETIREE("CF2")];
  const ctx = recap(lignes, ["CF1", "CF2"]);

  boutonSel(ctx.$, "Réactiver").listeners.click();
  const note = trouver(ctx.$("modal"), (n) => n.className === "danger-note");
  assert.notEqual(note, null, "l'avertissement doit être posé");
  assert.match(String(note.children[0]), /2026-08-01/, "la MEP concernée doit être nommée");
});

test("sans MEP gelée, pas d'avertissement", async () => {
  // Garde : un avertissement qui apparaît toujours n'avertit plus de rien.
  const ctx = recap([RETIREE("CF1")], ["CF1"]);
  boutonSel(ctx.$, "Réactiver").listeners.click();
  assert.equal(trouver(ctx.$("modal"), (n) => n.className === "danger-note"), null);
});

test("la perte du motif est annoncée", async () => {
  // `annuler_retrait` remet `retire` à None : le motif est effacé. L'action
  // est donc moins réversible qu'elle n'en a l'air, et ça se dit avant.
  const ctx = recap([RETIREE("CF1")], ["CF1"]);
  boutonSel(ctx.$, "Réactiver").listeners.click();
  const texte = (function tout(n) {
    if (typeof n === "string") return n;
    if (typeof n !== "object" || n === null) return "";
    return (n.children ?? []).map(tout).join(" ");
  })(ctx.$("modal"));
  assert.match(texte, /motif/i, `la fenêtre doit prévenir : ${texte}`);
});
