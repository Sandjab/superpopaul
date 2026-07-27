// Récapitulatif du plan : câblage de la barre de filtres.
//
// La table est reconstruite en entier à chaque changement de filtre — c'est
// voulu, elle affiche autre chose. Ce qui ne doit PAS l'être, c'est la saisie
// en cours : la recherche déclenche elle-même le re-rendu qui la détruirait.

const test = require("node:test");
const assert = require("node:assert");
const { chargerApp, trouver } = require("./dom_shim.js");

function ligne(cf, raison) {
  return {
    cf,
    participant: `iso6523-actorid-upis::0225:${cf}`,
    raison_sociale: raison,
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
  };
}

const LIGNES = [ligne("CF1", "Alpha"), ligne("CF2", "Bravo"), ligne("CF3", "Charlie")];

/** Écran de récapitulatif rendu, avec trois comptes. */
function recap(lignes = LIGNES) {
  const ctx = chargerApp();
  // Passer par `evaluer` : les objets du realm de test ne sont pas ceux du
  // contexte qui exécute `app.js`.
  ctx.evaluer("plan").lignes = ctx.evaluer(`(${JSON.stringify(lignes)})`);
  ctx.app.renderPlanRecap();
  return ctx;
}

const recherche = ($) => trouver($("plan-recap"), (n) => n.attrs?.type === "search");

/** Comptes affichés dans la table, dans l'ordre. */
function affiches($) {
  const out = [];
  (function descendre(n) {
    if (typeof n !== "object" || n === null) return;
    if (n.className === "cf") out.push(n.children[0]);
    for (const enfant of n.children ?? []) descendre(enfant);
  })($("plan-recap"));
  return out;
}

test("une recherche en cours survit au re-rendu qu'elle déclenche", () => {
  // Le champ est reconstruit par le rendu que sa propre frappe provoque : sans
  // valeur restaurée NI focus rendu, la lettre suivante tombe dans le vide et
  // la recherche repart de zéro à chaque caractère.
  const ctx = recap();

  const champ = recherche(ctx.$);
  champ.focus();
  champ.value = "Bravo";
  champ.listeners.input({ target: champ });

  const apres = recherche(ctx.$);
  assert.equal(apres.value, "Bravo", "la saisie doit rester affichée");
  assert.equal(ctx.evaluer("document.activeElement"), apres,
    "le champ reconstruit doit reprendre le focus, sinon la frappe suivante est perdue");
});

test("la recherche filtre bien la table", () => {
  // Garde : préserver la saisie ne doit pas se payer d'un rendu qui n'a plus
  // lieu — c'est le filtrage qui justifie de reconstruire la table.
  const ctx = recap();

  const champ = recherche(ctx.$);
  champ.value = "Bravo";
  champ.listeners.input({ target: champ });

  assert.deepEqual(affiches(ctx.$), ["CF2"], "seul le compte cherché doit rester");
});

test("la recherche porte aussi sur le compte et l'adressage", () => {
  const ctx = recap();

  const champ = recherche(ctx.$);
  champ.value = "cf3";
  champ.listeners.input({ target: champ });

  assert.deepEqual(affiches(ctx.$), ["CF3"], "la recherche est insensible à la casse");
});

test("le curseur reste où il était dans la recherche reconstruite", () => {
  // Vécu en application : taper « abc » écrivait « cba ». Le champ neuf porte
  // sa valeur par ATTRIBUT, qui ne déplace pas le curseur — il reste à zéro, et
  // chaque frappe s'insère devant la précédente.
  //
  // Curseur au MILIEU volontairement : le remettre en fin de texte passerait
  // pour une correction alors qu'il déplacerait la saisie de qui reprend le
  // début de sa recherche.
  const ctx = recap();

  const champ = recherche(ctx.$);
  champ.focus();
  champ.value = "Bravo";
  champ.setSelectionRange(2, 2);
  champ.listeners.input({ target: champ });

  const apres = recherche(ctx.$);
  assert.equal(apres.selectionStart, 2, "le curseur doit être rendu là où il était");
  assert.equal(apres.selectionEnd, 2);
});
