// Faux DOM minimal pour exécuter le VRAI `src/app.js` hors navigateur.
//
// Le frontend est en vanilla sans bundler : il n'y a pas de runner à hériter,
// et les bugs qu'on y trouve sont des bugs de câblage (un champ reconstruit
// qui perd sa valeur, un écouteur qui ne rebranche pas). Ce shim ne modélise
// donc que ce dont `app.js` se sert, mais il est FIDÈLE sur le point qui les
// révèle : un élément neuf n'a que la valeur portée par son attribut, la
// saisie de l'utilisateur ne le suit pas.
//
// Volontairement hors de portée : layout, styles, événements qui bouillonnent,
// sélecteurs CSS. Un test qui en aurait besoin doit passer par l'application.

const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

// Les scripts de `index.html`, dans son ordre, hors vendor (Sortable, stubé
// plus bas) : ils partagent une seule portée globale dans la page, et une
// fonction d'un fichier en appelle une autre sans cérémonie — n'en charger
// qu'un donnerait des ReferenceError que la vraie application n'a pas.
const SCRIPTS = ["app.js", "columns.js", "cockpit.js"]
  .map((f) => path.join(__dirname, "..", "src", f));

function creerDocument() {
  // `document` est nommé ici pour que `focus()` puisse y poser activeElement.
  let document;
  // `getElementById` rend le DERNIER élément créé sous cet id, comme le vrai
  // DOM rend celui qui est attaché après un remplacement de sous-arbre.
  const parId = new Map();

  function createElement(tag) {
    const el = {
      tagName: tag,
      attrs: {},
      listeners: {},
      children: [],
      style: {},
      // Booléen dès le départ, comme dans un vrai DOM : laissé à `undefined`,
      // il rendait « bouton rouvert » indiscernable de « jamais touché ».
      disabled: false,
      // Curseur de saisie. Zéro sur un élément neuf, et `setAttribute("value")`
      // ne l'en bouge PAS — c'est l'attribut de contenu, pas la propriété
      // `value` (qui, elle, pousse le curseur en fin de texte). D'où un champ
      // reconstruit qui, sans rien de plus, insère la frappe suivante au début.
      selectionStart: 0,
      selectionEnd: 0,
      setSelectionRange(debut, fin) { this.selectionStart = debut; this.selectionEnd = fin; },
      dataset: {},
      _classes: [],
      setAttribute(k, v) {
        this.attrs[k] = String(v);
        if (k === "id") parId.set(String(v), this);
      },
      getAttribute(k) { return this.attrs[k]; },
      removeAttribute(k) { delete this.attrs[k]; },
      addEventListener(evt, fn) { this.listeners[evt] = fn; },
      append(...kids) { for (const k of kids) if (k != null) this.children.push(k); },
      replaceChildren(...kids) { this.children = []; this.append(...kids); },
      querySelectorAll: () => [],
      /** Vrai pour l'élément lui-même et toute sa descendance, comme le DOM. */
      contains(autre) {
        if (autre === this) return true;
        return this.children.some((c) => typeof c === "object" && c !== null && c.contains?.(autre));
      },
      focus() { document.activeElement = this; },
      /** Déclenche le clic AVEC un événement, comme le vrai DOM : un écouteur
       *  qui lit `ev.currentTarget` (pour agir sur le bouton cliqué) explosait
       *  quand un test appelait `listeners.click()` à la main. Rend ce que
       *  l'écouteur rend, pour que les tests puissent l'attendre. */
      click() { return this.listeners.click?.({ target: this, currentTarget: this }); },
      remove() {},
    };
    // `textContent` est la vue texte des enfants, comme dans un vrai DOM :
    // `h(tag, {}, "libellé")` empile une chaîne, et `el.textContent = "…"` la
    // remplace. En faire un champ indépendant rendait les deux écritures
    // invisibles l'une à l'autre — un libellé posé par `h()` restait illisible.
    Object.defineProperty(el, "textContent", {
      get() {
        return this.children
          .map((c) => (typeof c === "string" ? c : c?.textContent ?? ""))
          .join("");
      },
      set(v) { this.children = [String(v)]; },
    });
    // `className` et `classList` sont deux vues du MÊME état, comme dans un
    // vrai DOM : `app.js` pose l'un (via `h()`) et lit l'autre (`toggle`,
    // `contains`). Les stuber vides rendait indiscernables « la classe est
    // posée » et « elle ne l'est pas ».
    Object.defineProperty(el, "className", {
      get() { return this._classes.join(" "); },
      set(v) { this._classes = String(v).split(/\s+/).filter(Boolean); },
    });
    el.classList = {
      add(...cs) { for (const c of cs) if (!el._classes.includes(c)) el._classes.push(c); },
      remove(...cs) { el._classes = el._classes.filter((x) => !cs.includes(x)); },
      toggle(c, force) {
        const poser = force === undefined ? !el._classes.includes(c) : !!force;
        if (poser) el.classList.add(c); else el.classList.remove(c);
        return poser;
      },
      contains: (c) => el._classes.includes(c),
    };
    Object.defineProperty(el, "value", {
      get() {
        if (this._value !== undefined) return this._value;
        // Un <select> tire sa valeur de son option sélectionnée, la première
        // à défaut — c'est ainsi qu'`app.js` restaure la forme de rampe.
        if (this.tagName === "select") {
          const opt = this.children.find((c) => c.selected) ?? this.children[0];
          return opt ? (opt.attrs.value ?? "") : "";
        }
        return this.attrs.value ?? "";
      },
      set(v) { this._value = String(v); },
    });
    // `setAttribute("checked", false)` COCHE la case dans un vrai navigateur :
    // l'attribut compte par sa présence. Le shim reproduit ce piège, c'est lui
    // qui a fait passer la case « pilote » pour décochée.
    Object.defineProperty(el, "checked", {
      get() { return this._checked !== undefined ? this._checked : "checked" in this.attrs; },
      set(v) { this._checked = !!v; },
    });
    return el;
  }

  // Un id inconnu rend un élément factice : `app.js` câble au chargement des
  // écouteurs sur des ids qui vivent dans index.html, absent ici.
  function getElementById(id) {
    if (!parId.has(id)) {
      const el = createElement("div");
      el.setAttribute("id", id);
    }
    return parId.get(id);
  }

  document = {
    createElement,
    getElementById,
    /** Élément ayant le focus, comme dans un navigateur : null par défaut. */
    activeElement: null,
    // Les sélecteurs CSS ne sont pas modélisés : un test qui en dépendrait
    // porterait sur le rendu réel, hors de portée de ce shim. `querySelector`
    // rend quand même un élément jetable — comme `getElementById` — pour que le
    // code qui pose une propriété sur son résultat traverse sans exploser.
    querySelectorAll: () => [],
    querySelector: () => createElement("div"),
    addEventListener() {},
    body: createElement("body"),
  };
  return document;
}

/** Charge `src/app.js` dans un contexte NEUF (aucun état partagé entre tests).
 *
 *  Rend :
 *  - `app`  : les fonctions déclarées par app.js (`renderPlanAside`, `planParams`…) ;
 *  - `$`    : `getElementById` du faux document ;
 *  - `invocations` : les `invoke("commande", args)` partis vers le backend ;
 *  - `evaluer(expr)` : évalue une expression DANS le contexte. Sert à atteindre
 *    l'état déclaré en `const` (`plan`, `state`) : ces liaisons vivent dans
 *    l'environnement lexical du realm, pas sur `globalThis`, donc `app.plan`
 *    est indéfini là où `evaluer("plan")` rend l'objet — et le mute. */
function chargerApp() {
  const document = creerDocument();
  const invocations = [];
  // `app.js` fait `const { invoke } = window.__TAURI__.core` au chargement :
  // remplacer la fonction après coup n'aurait aucun effet. D'où l'indirection,
  // que `repondreAux` réarme quand un test veut simuler le backend.
  let repondre = () => null;
  const invoke = (cmd, args) => {
    invocations.push([cmd, args]);
    return Promise.resolve(repondre(cmd, args));
  };
  const window = {
    __TAURI__: {
      core: { invoke },
      event: { listen: () => Promise.resolve(() => {}) },
      dialog: { open: async () => null, save: async () => null },
      // `cockpit.js` s'abonne à la fermeture de fenêtre dès le chargement, et
      // enchaîne un `.catch` sur la promesse rendue par `onCloseRequested`.
      window: {
        getCurrentWindow: () => ({
          onCloseRequested: () => Promise.resolve(() => {}),
          destroy() {},
        }),
      },
      app: { getVersion: () => Promise.resolve("test") },
      opener: { openUrl: async () => {} },
    },
    addEventListener() {},
    document,
  };

  // Les minuteries d'`app.js` (anti-rebond du recalcul) ne doivent pas retenir
  // le process de test : `unref` les laisse fonctionner sans le maintenir en vie.
  const minuterie = (pose) => (fn, ms) => { const t = pose(fn, ms); t.unref?.(); return t; };

  // Console captée : un `catch` qui se contente d'un `console.warn` rend une
  // garde manquante indiscernable d'une erreur avalée. Les tests peuvent donc
  // exiger une sortie propre, pas seulement une absence d'effet.
  const plaintes = [];
  const console_ = {
    ...console,
    warn: (...a) => plaintes.push(["warn", a.join(" ")]),
    error: (...a) => plaintes.push(["error", a.join(" ")]),
  };

  const ctx = vm.createContext({
    document, window, console: console_,
    setTimeout: minuterie(setTimeout), clearTimeout,
    setInterval: minuterie(setInterval), clearInterval,
    Promise, JSON, Math, Date, Intl, Number, String, Array, Object, Set, Map, Error,
    TextEncoder, TextDecoder, structuredClone,
  });
  ctx.globalThis = ctx;
  // Seule dépendance vendorisée du front (drag des colonnes) : stubée, aucun
  // test ne porte sur le drag, qui ne se rejoue pas sans pointeur.
  ctx.Sortable = function Sortable() { return { destroy() {} }; };
  ctx.Sortable.create = () => ({ destroy() {} });
  for (const s of SCRIPTS) {
    vm.runInContext(fs.readFileSync(s, "utf8"), ctx, { filename: path.basename(s) });
  }

  return {
    app: ctx,
    $: document.getElementById,
    invocations,
    /** `["warn"|"error", message]` émis par le code testé. */
    plaintes,
    evaluer: (expr) => vm.runInContext(expr, ctx),
    /** Installe la réponse du backend : `(commande, args) => valeur`. */
    repondreAux: (fn) => { repondre = fn; },
  };
}

/** Premier ÉLÉMENT du sous-arbre satisfaisant `predicat`, en profondeur
 *  d'abord. Les enfants texte sont ignorés : `h()` les empile tels quels, et un
 *  prédicat qui lit `attrs` ou `tagName` s'y casserait. */
function trouver(noeud, predicat) {
  if (typeof noeud !== "object" || noeud === null) return null;
  if (predicat(noeud)) return noeud;
  for (const enfant of noeud.children ?? []) {
    const t = trouver(enfant, predicat);
    if (t) return t;
  }
  return null;
}

module.exports = { chargerApp, trouver };
