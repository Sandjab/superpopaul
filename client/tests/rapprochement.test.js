// Écran de rapprochement du plan avec le fichier ouvert.
//
// Le cœur métier (rapprochement.rs) est déjà livré et testé côté Rust : ces
// tests ne portent que sur le câblage JS — l'empreinte qui doit voyager
// intacte jusqu'à l'application (sans quoi le backend refuse, pour une raison
// que l'utilisateur ne peut pas deviner), le déclencheur qui reste inerte
// sans écart, et l'affichage d'une erreur backend sans jamais passer par
// innerHTML (comptes, motifs et messages viennent tous d'entrées non fiables).

const test = require("node:test");
const assert = require("node:assert/strict");
const { chargerApp, trouver } = require("./dom_shim");

function ligne(cf, extra = {}) {
  return {
    cf, participant: `iso6523-actorid-upis::0225:${cf}`, raison_sociale: `Société ${cf}`,
    jj: 5, pa: "Cegedim", mep_id: 1, mep_date: "2026-09-01", run_num: "RF01",
    run_date: "2026-09-10", origine: "auto", etat: "eligible", gelee: false,
    retire_motif: null, ...extra,
  };
}

const ECART = {
  cf: "CF1", gelee: false,
  nature: { type: "eligibilite_perdue", avant: "CTC prêt", apres: "CTC prêt plus tard" },
  action: { type: "retirer", motif: "Rapprochement du 28/07/2026 — CTC prêt plus tard" },
};

/** Récap prêt pour le rapprochement : un plan déjà généré, une ligne. */
function ecran() {
  const ctx = chargerApp();
  const p = ctx.evaluer("plan");
  p.genere = true;
  p.rapportFichier = "identique";
  p.lignes = ctx.evaluer(`(${JSON.stringify([ligne("CF1")])})`);
  ctx.app.renderPlanRecap();
  return ctx;
}

const boutonRapprocher = ($) => trouver($("plan-recap"),
  (n) => n.tagName === "button" && String(n.children[0] ?? "") === "Rapprocher…");

const boutonModale = ($, debut) => trouver($("modal"),
  (n) => n.tagName === "button" && String(n.children[0] ?? "").startsWith(debut));

test("l'empreinte survit à un re-rendu de l'écran", async () => {
  const ctx = ecran();
  const EMPREINTE = "3f2a9c...empreinte-connue";
  ctx.repondreAux((cmd) => {
    if (cmd === "plan_rapprocher")
      return ctx.evaluer(`(${JSON.stringify({
        rapprochement: { ecarts: [ECART], inchangees: 0, avertissements: [] },
        empreinte: EMPREINTE,
        annuaire_incomplet: null,
      })})`);
    if (cmd === "plan_lignes") return ctx.evaluer(`(${JSON.stringify([ligne("CF1")])})`);
    if (cmd === "plan_rapprocher_appliquer")
      return ctx.evaluer(`(${JSON.stringify({ obsoletes: [], rapport: "" })})`);
    return null;
  });

  await boutonRapprocher(ctx.$).click();
  // Un re-rendu du récap (filtre changé, sélection modifiée…) ne doit rien
  // effacer de ce que la revue retient : la modale vit hors de `#plan-recap`,
  // que `renderPlanRecap` reconstruit entièrement.
  ctx.app.renderPlanRecap();

  await boutonModale(ctx.$, "Appliquer").click();

  const appel = ctx.invocations.find(([c]) => c === "plan_rapprocher_appliquer");
  assert.notEqual(appel, undefined, "la commande d'application doit être appelée");
  assert.equal(appel[1].empreinte, EMPREINTE,
    "sans elle, le backend refuse et l'utilisateur lit une erreur incompréhensible");
});

test("sans écart, le déclencheur d'application reste inerte", async () => {
  const ctx = ecran();
  ctx.repondreAux((cmd) => {
    if (cmd === "plan_rapprocher")
      return ctx.evaluer(`(${JSON.stringify({
        rapprochement: { ecarts: [], inchangees: 2001, avertissements: [] },
        empreinte: "peu importe ici",
        annuaire_incomplet: null,
        retraits_manuels: 0,
      })})`);
    return null;
  });

  await boutonRapprocher(ctx.$).click();

  const appliquer = boutonModale(ctx.$, "Appliquer");
  assert.ok(appliquer, "un bouton Appliquer doit exister, ne serait-ce que désactivé");
  assert.equal(appliquer.disabled, true, "aucun écart : rien à appliquer");
  appliquer.click();
  assert.equal(ctx.invocations.find(([c]) => c === "plan_rapprocher_appliquer"), undefined,
    "sans écart, l'application ne doit jamais partir vers le backend");
});

test("sans écart mais avec des retraits manuels, l'application produit la note", async () => {
  // Cas réel : on retire des comptes à la main, on relance une résolution qui
  // ne fait basculer aucun verdict, on rapproche. Zéro écart — et pourtant le
  // lot partirait sans note alors que trois comptes ont quitté les fichiers.
  const ctx = ecran();
  const CHEMIN = "/sortie/brm2608_rapprochement_2026-08-14_143207.html";
  ctx.repondreAux((cmd) => {
    if (cmd === "plan_rapprocher")
      return ctx.evaluer(`(${JSON.stringify({
        rapprochement: { ecarts: [], inchangees: 2001, avertissements: [] },
        empreinte: "peu importe ici",
        annuaire_incomplet: null,
        retraits_manuels: 3,
      })})`);
    if (cmd === "plan_lignes") return ctx.evaluer(`(${JSON.stringify([ligne("CF1")])})`);
    if (cmd === "plan_rapprocher_appliquer")
      return ctx.evaluer(`(${JSON.stringify({ obsoletes: [], rapport: CHEMIN })})`);
    return null;
  });

  await boutonRapprocher(ctx.$).click();

  const produire = boutonModale(ctx.$, "Produire la note");
  assert.ok(produire, "le déclencheur doit changer de libellé, pas rester « Appliquer »");
  assert.equal(produire.disabled, false, "il y a quelque chose à écrire");
  await produire.click();

  assert.ok(ctx.invocations.find(([c]) => c === "plan_rapprocher_appliquer"),
    "l'application doit partir vers le backend");
  const texte = ctx.$("plan-banner").textContent;
  assert.match(texte, /3 retrait/, "le compte rendu doit dire ce qui a été documenté");
  assert.match(texte, /brm2608_rapprochement_2026-08-14_143207\.html/,
    "le bandeau nomme le rapport : c'est la seule trace du livrable");
});

test("le compte rendu nomme le rapport écrit", async () => {
  // Une fois la modale fermée, le bandeau est la seule trace du livrable :
  // sans son nom, l'utilisateur ne sait pas qu'un document est parti avec
  // les fichiers, ni lequel retrouver dans le répertoire de sortie.
  const ctx = ecran();
  const CHEMIN = "/data/sortie/brm2607_rapprochement_2026-07-28_143205.html";
  ctx.repondreAux((cmd) => {
    if (cmd === "plan_rapprocher")
      return ctx.evaluer(`(${JSON.stringify({
        rapprochement: { ecarts: [ECART], inchangees: 0, avertissements: [] },
        empreinte: "peu importe ici",
        annuaire_incomplet: null,
      })})`);
    if (cmd === "plan_lignes") return ctx.evaluer(`(${JSON.stringify([ligne("CF1")])})`);
    if (cmd === "plan_rapprocher_appliquer")
      return ctx.evaluer(`(${JSON.stringify({ obsoletes: [], rapport: CHEMIN })})`);
    return null;
  });

  await boutonRapprocher(ctx.$).click();
  await boutonModale(ctx.$, "Appliquer").click();

  const texte = ctx.$("plan-banner").textContent;
  assert.match(texte, /brm2607_rapprochement_2026-07-28_143205\.html/,
    "le bandeau doit nommer le rapport");
  assert.ok(!texte.includes("/data/sortie/"),
    "le chemin de la machine n'apprend rien : seul le nom compte");
});

test("une erreur backend s'affiche telle quelle, jamais via innerHTML", async () => {
  const ctx = ecran();
  const MESSAGE = "le fichier a changé depuis le calcul <script>alert(1)</script>";
  ctx.repondreAux((cmd) => {
    if (cmd === "plan_rapprocher") throw MESSAGE;
    return null;
  });

  await boutonRapprocher(ctx.$).click();

  const banniere = ctx.$("plan-banner");
  assert.equal(banniere.textContent, MESSAGE, "le message doit être posé tel quel, sans interprétation");
  assert.equal(trouver(banniere, (n) => n.tagName === "script"), null,
    "aucun nœud <script> ne doit apparaître dans le DOM : la donnée n'est pas fiable");
});

/** Tous les nœuds dont la classe contient "rappro-avert", dans l'ordre du DOM
 *  (profondeur d'abord) — la revue en pose au plus deux : celui de l'annuaire
 *  (gravité supérieure) et celui du calcul. */
function boitesAvertissement($) {
  const out = [];
  (function marcher(n) {
    if (typeof n !== "object" || n === null) return;
    if (typeof n.className === "string" && n.className.includes("rappro-avert")) out.push(n);
    for (const c of n.children ?? []) marcher(c);
  })($("modal"));
  return out;
}

test("l'avertissement d'annuaire est séparé de ceux du calcul, et passe en tête", async () => {
  // Régression du 28/07/2026 : les deux étaient fondus dans le même tableau
  // de chaînes côté backend. Un « 0 éligibilité perdue » peut alors vouloir
  // dire « l'annuaire ne sait pas les voir » — la distinction doit rester
  // visible à l'écran, pas seulement dans la forme de la réponse.
  const ctx = ecran();
  const TEXTE_ANNUAIRE = "l'annuaire PPF a été construit par cumul de 2 fichiers : une "
    + "éligibilité PPF perdue n'y est pas détectable.";
  const TEXTE_CALCUL = "ce rapprochement retire 47 des 2001 lignes actives du plan";
  ctx.repondreAux((cmd) => {
    if (cmd === "plan_rapprocher")
      return ctx.evaluer(`(${JSON.stringify({
        rapprochement: { ecarts: [ECART], inchangees: 0, avertissements: [TEXTE_CALCUL] },
        empreinte: "peu importe ici",
        annuaire_incomplet: TEXTE_ANNUAIRE,
      })})`);
    return null;
  });

  await boutonRapprocher(ctx.$).click();

  const boites = boitesAvertissement(ctx.$);
  assert.equal(boites.length, 2, "un bloc pour l'annuaire, un pour le calcul");
  assert.match(boites[0].className, /rappro-avert-hard/,
    "l'avertissement d'annuaire doit porter le style de gravité supérieure");
  assert.match(boites[0].textContent, /annuaire PPF/, "et paraître en premier, avant celui du calcul");
  assert.ok(!boites[1].className.includes("rappro-avert-hard"),
    "celui du calcul reste au style normal");
  assert.match(boites[1].textContent, /retire 47/);
});
