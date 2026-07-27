// Fichiers d'une génération précédente supprimés du répertoire de livraison.
//
// Le backend fait le ménage — un fichier de MEP périmé peut être transmis par
// erreur — mais ce qu'on retire d'un répertoire de livraison ne s'efface pas en
// silence. Ces tests vérifient que l'écran le DIT, quel que soit le geste qui a
// provoqué le ménage.

const test = require("node:test");
const assert = require("node:assert");
const { chargerApp } = require("./dom_shim.js");

/** Texte affiché par le bandeau du plan (`planBanner` pose un nœud texte). */
function bandeau($) {
  const el = $("plan-banner");
  return { classe: el.className, texte: el.children.join("") };
}

const OBSOLETES = [
  "/data/sortie/brm_plan_mep_4_2026-12-01.txt",
  "/data/sortie/brm_plan_mep_5_2027-01-01.txt",
];

/** `PlanApercu` minimal : ce que les rendus déclenchés par la génération lisent. */
const APERCU = {
  funnel: { lignes: 10, cf_distincts: 10, jj_valide: 10, resolus: 10,
            ctc_ready: 10, ppf_usable: 10, eligibles: 10 },
  timeline: [], stock_jj: [], plateformes: [], avertissements: [],
  meps: ["2026-09-01"], cible: 10, total: 10, geles: 0, epingles: 0, retires: 0,
};

test("la génération annonce les fichiers supprimés", () => {
  const ctx = chargerApp();
  ctx.repondreAux((cmd) => {
    if (cmd === "plan_generate") {
      return ctx.evaluer(`(${JSON.stringify({
        apercu: APERCU,
        fichiers: [],
        obsoletes: OBSOLETES,
      })})`);
    }
    // `rechargerRecap` enchaîne : sans réponse, il casse avant l'assertion.
    if (cmd === "plan_lignes") return ctx.evaluer("[]");
    return null;
  });

  return ctx.app.genererPlan().then(() => {
    const b = bandeau(ctx.$);
    assert.match(b.texte, /2 fichier/, `bandeau : ${b.texte}`);
    assert.match(b.texte, /brm_plan_mep_4_2026-12-01\.txt/,
      "les fichiers retirés doivent être nommés, pas seulement comptés");
    assert.ok(!b.texte.includes("/data/sortie/"),
      "le chemin complet noierait le message : seul le nom du fichier est utile");
  });
});

test("une retouche annonce aussi les fichiers supprimés", () => {
  // Un retrait peut vider une MEP : son fichier n'a alors plus lieu d'être. Le
  // ménage n'est pas réservé à la génération, l'annonce non plus.
  const ctx = chargerApp();
  ctx.evaluer("plan").sel = ctx.evaluer("new Set(['CF1'])");
  ctx.repondreAux((cmd) => {
    if (cmd === "plan_retirer") return ctx.evaluer(`(${JSON.stringify(OBSOLETES.slice(0, 1))})`);
    if (cmd === "plan_lignes") return ctx.evaluer("[]");
    return null;
  });

  ctx.app.ouvrirRetrait();
  const modale = ctx.$("modal");
  const bouton = (function chercher(n) {
    if (typeof n !== "object" || n === null) return null;
    if (n.tagName === "button" && String(n.children[0] ?? "").startsWith("Retirer ")) return n;
    for (const e of n.children ?? []) { const t = chercher(e); if (t) return t; }
    return null;
  })(modale);
  assert.ok(bouton, "le bouton de retrait doit exister");

  return bouton.listeners.click().then(() => {
    assert.match(bandeau(ctx.$).texte, /brm_plan_mep_4_2026-12-01\.txt/);
  });
});

test("sans fichier supprimé, aucun bandeau n'est posé", () => {
  // Garde : annoncer « 0 fichier supprimé » à chaque génération noierait les
  // messages qui comptent.
  const ctx = chargerApp();
  ctx.repondreAux((cmd) => {
    if (cmd === "plan_generate") {
      return ctx.evaluer(`(${JSON.stringify({
        apercu: APERCU,
        fichiers: [],
        obsoletes: [],
      })})`);
    }
    // `rechargerRecap` enchaîne : sans réponse, il casse avant l'assertion.
    if (cmd === "plan_lignes") return ctx.evaluer("[]");
    return null;
  });

  return ctx.app.genererPlan().then(() => {
    assert.equal(bandeau(ctx.$).classe, "hidden", "le bandeau doit rester fermé");
  });
});
