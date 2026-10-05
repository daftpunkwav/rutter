/**
 * @fileoverview In-page DOM serializer for rutter accessibility snapshots.
 *
 * Walks the composed DOM (including open shadow roots and slot
 * assignments) and reports a plain JSON tree following the snapshot
 * format specification. Hidden elements are omitted, never guessed at. The
 * script never throws: every per-node computation is guarded and a
 * failure degrades that node to role "generic".
 *
 * The caller evaluates this script inside a page and converts the
 * returned envelope with rutter-observe. Owned by rutter-observe;
 * engine and orchestration code must not modify it.
 */
(() => {
  'use strict';

  const MAX_NODES = 50000;
  const MAX_DEPTH = 200;
  const NAME_LIMIT = 120;
  const VALUE_LIMIT = 200;
  // How many reverse-store entries may pile up before a snapshot sweeps
  // the collected ones out. See pruneRefs.
  const REF_SWEEP_AT = 512;

  // Mints the scope every ref of one document carries.
  //
  // Each document counts its refs from 1, so two pages that were both
  // snapshotted can hold the same `e17`. The resolver looks a
  // reference up in whichever page is active and reports `missing`
  // only when the store has no such ref, so after switching tabs a
  // stale `e17` would silently resolve to the other page's own
  // `e17` -- the wrong element, with no error. Suffixing one random
  // scope shared by all of a document's refs (the store's lifetime is
  // the document's) makes a cross-page hit need the counter and the
  // scope to collide at once, while same-page refs keep the stability
  // the snapshot format promises.
  //
  // The scope has to be a value the page cannot choose, and this script
  // runs after the page's own, so it cannot be minted here: `Math.random`,
  // `crypto`, and the prototypes a conversion would go through are all
  // replaceable. `__rutterRefScope` is that minter, installed and locked
  // before the document ran anything -- at document start for a document
  // the engine loads, and from an isolated world for one that was already
  // loaded when rutter attached (see `entropy.js` and
  // `crates/engine-cdp`). There is no in-page fallback on purpose: a
  // scope drawn from a source the page can replace would hand two
  // documents the same scope, which is the collision this exists to
  // prevent. A document with no minter gets the degradation every other
  // store failure gets -- `ensureRefStore` reports the snapshot truncated
  // and hands out no references.
  function mintScope() {
    return window.__rutterRefScope();
  }

  // The lookup tables below are a Set or a Map, never an object
  // literal. Every key they are asked about comes from the page -- a
  // tag name, an author-supplied `role` -- and an object literal also
  // answers for `constructor`, `toString`, and the rest of
  // `Object.prototype`, so a page naming one of those as its role
  // would read as actionable, or have its content taken for site
  // chrome. A Set holds exactly what was put in it.
  const SKIPPED_TAGS = new Set([
    'SCRIPT', 'STYLE', 'NOSCRIPT', 'TEMPLATE', 'HEAD',
    'META', 'LINK', 'TITLE', 'BR', 'BASE', 'DATALIST'
  ]);

  const IMPLICIT_ROLES = new Map([
    ['A', 'link'], ['AREA', 'link'], ['BUTTON', 'button'],
    ['SELECT', 'combobox'], ['OPTION', 'option'], ['OPTGROUP', 'group'],
    ['TEXTAREA', 'textbox'], ['H1', 'heading'], ['H2', 'heading'],
    ['H3', 'heading'], ['H4', 'heading'], ['H5', 'heading'],
    ['H6', 'heading'], ['IMG', 'image'], ['NAV', 'navigation'],
    ['UL', 'list'], ['OL', 'list'], ['LI', 'listitem'],
    ['TABLE', 'table'], ['TR', 'row'], ['TD', 'cell'],
    ['TH', 'columnheader'], ['FORM', 'form'], ['MAIN', 'main'],
    ['HEADER', 'banner'], ['FOOTER', 'contentinfo'],
    ['ASIDE', 'complementary'], ['DIALOG', 'dialog'],
    ['PROGRESS', 'progressbar'], ['DETAILS', 'group'],
    ['SUMMARY', 'button'], ['HR', 'separator'], ['OUTPUT', 'status'],
    ['METER', 'meter']
  ]);

  const ACTIONABLE_ROLES = new Set([
    'button', 'link', 'textbox', 'searchbox', 'checkbox',
    'radio', 'combobox', 'listbox', 'option', 'menuitem',
    'tab', 'slider', 'spinbutton', 'switch', 'treeitem'
  ]);

  // Roles whose accessible name may come from their text content.
  // Containers (generic, list, group, ...) must not inherit the page
  // text: their names stay empty like the ARIA spec requires.
  const TEXT_NAME_ROLES = new Set([
    'heading', 'button', 'link', 'listitem', 'option',
    'cell', 'columnheader', 'rowheader', 'menuitem', 'tab',
    'treeitem', 'term', 'definition', 'alert', 'status',
    'paragraph', 'caption', 'code', 'emphasis', 'strong',
    'time', 'blockquote', 'note', 'tooltip'
  ]);

  const state = { count: 0, truncated: false };

  // Forgets the refs whose element the page has already collected.
  //
  // `reverse` is a strong Map: its key is the ref string and its value a
  // WeakRef, so nothing in it is ever collectible while the entry lives.
  // Without a sweep the store grows with every element the page has ever
  // shown, and a long-lived document that re-renders (a virtual list, an
  // infinite feed) would grow the browser process with it for as long as
  // the tab stays open.
  //
  // Dropping a collected element's entry answers nothing differently:
  // the resolver dereferences every entry and reports `missing` when the
  // element is gone, so the ref was already unresolvable. Refs of
  // elements that are still alive keep their entry, and the WeakMap
  // hands them the same ref again, which is the stability the snapshot
  // format promises.
  function pruneRefs(store) {
    if (!store?.reverse ||
        typeof store.reverse.forEach !== 'function' ||
        typeof store.reverse.size !== 'number') return;
    if (store.reverse.size < REF_SWEEP_AT) return;
    const collected = [];
    store.reverse.forEach((weak, ref) => {
      if (!weak || typeof weak.deref !== 'function' || weak.deref() === undefined) {
        collected.push(ref);
      }
    });
    for (const ref of collected) {
      store.reverse.delete(ref);
    }
  }

  function ensureRefStore() {
    try {
      if (!window.__rutterRefStore) {
        // `map` mints refs (element -> ref); `reverse` resolves them
        // (ref -> WeakRef<element>) for the action executor. WeakRefs
        // let garbage-collected elements resolve to nothing. `scope`
        // namespaces every ref this document mints; see mintScope.
        window.__rutterRefStore = {
          map: new WeakMap(),
          reverse: new Map(),
          counter: 0,
          scope: mintScope()
        };
      }
      pruneRefs(window.__rutterRefStore);
      return window.__rutterRefStore;
    } catch {
      state.truncated = true;
      return null;
    }
  }

  function inputRole(el) {
    let type = 'text';
    try {
      const explicit = el.getAttribute('type');
      if (explicit) type = explicit.toLowerCase();
    } catch {}
    if (type === 'checkbox') return 'checkbox';
    if (type === 'radio') return 'radio';
    if (type === 'button' || type === 'submit' || type === 'reset') return 'button';
    if (type === 'range') return 'slider';
    if (type === 'number') return 'spinbutton';
    if (type === 'search') return 'searchbox';
    return 'textbox';
  }

  function roleOf(el) {
    try {
      const explicit = el.getAttribute('role');
      if (explicit) {
        const first = explicit.trim().split(/\s+/)[0];
        if (first) return first;
      }
    } catch {}
    if (el.tagName === 'INPUT') return inputRole(el);
    return IMPLICIT_ROLES.get(el.tagName) || 'generic';
  }

  function isVisible(el) {
    try {
      const rects = el.getClientRects();
      if (!rects || rects.length === 0) return false;
      const style = window.getComputedStyle(el);
      return style.visibility !== 'hidden' && style.visibility !== 'collapse';
    } catch {
      return false;
    }
  }

  function clip(text, limit) {
    if (text.length <= limit) return text;
    let sliced = text.slice(0, limit - 3);
    // Never cut between a surrogate pair: a lone surrogate breaks
    // JSON consumers. Drop the orphaned high surrogate if present.
    const last = sliced.charCodeAt(sliced.length - 1);
    if (last >= 0xd800 && last <= 0xdbff) sliced = sliced.slice(0, -1);
    return sliced + '...';
  }

  // The accessible name of a text-name role may fold the whole
  // subtree's textContent, and the clip then keeps 120 characters of
  // it. Folding a text-dense subtree (a listitem or blockquote holding
  // the page's main content) pays for megabytes the clip throws away
  // on every snapshot, so the fold runs on a raw window that grows
  // only while the folded prefix is still too short to clip. A folded
  // prefix of the raw text is always a prefix of the folded whole (a
  // run of whitespace cut in half collapses to the same single space
  // or trims away), so once the window's fold reaches NAME_LIMIT the
  // clip below is the clip the full text would have produced. Only a
  // pathological mostly-whitespace subtree keeps growing the window;
  // the doubling stays bounded by the full text, and the last round
  // folds all of it, which is the answer the old fold gave.
  function textNameOf(el) {
    const text = el.textContent || '';
    let span = NAME_LIMIT * 2;
    for (;;) {
      const folded = text.slice(0, span).replace(/\s+/g, ' ').trim();
      if (folded.length >= NAME_LIMIT || span >= text.length) {
        return folded ? clip(folded, NAME_LIMIT) : null;
      }
      span *= 2;
    }
  }

  function nameOf(el, role) {
    try {
      const label = el.getAttribute('aria-label');
      if (label?.trim()) return clip(label.replace(/\s+/g, ' ').trim(), NAME_LIMIT);
      if (role === 'image') {
        const alt = el.getAttribute('alt');
        if (alt?.trim()) return clip(alt.replace(/\s+/g, ' ').trim(), NAME_LIMIT);
      }
      if (el.labels && el.labels.length > 0) {
        const labelText = (el.labels[0].textContent || '').replace(/\s+/g, ' ').trim();
        if (labelText) return clip(labelText, NAME_LIMIT);
      }
      const placeholder = el.getAttribute('placeholder');
      if (placeholder?.trim()) return clip(placeholder, NAME_LIMIT);
      const title = el.getAttribute('title');
      if (title?.trim()) return clip(title, NAME_LIMIT);
      if (!TEXT_NAME_ROLES.has(role)) return null;
      return textNameOf(el);
    } catch {
      return null;
    }
  }

  // The value reader for form controls. Spelled `valueTextOf` so the
  // function does not shadow `Object.prototype.valueOf` in this scope.
  function valueTextOf(el) {
    try {
      const tag = el.tagName;
      if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') {
        const value = String(el.value || '');
        return value ? clip(value, VALUE_LIMIT) : null;
      }
    } catch {}
    return null;
  }

  // The role decides whether `checked` means anything; the caller has
  // already computed it, so re-deriving it from the element here would
  // repeat the attribute walk on every serialized node.
  function checkedOf(el, role) {
    try {
      if (role === 'checkbox' || role === 'radio') return Boolean(el.checked);
    } catch {}
    return null;
  }

  function disabledOf(el) {
    try {
      if (el.disabled) return true;
      const aria = el.getAttribute('aria-disabled');
      return aria === 'true';
    } catch {
      return false;
    }
  }

  function rectOf(el) {
    try {
      const box = el.getBoundingClientRect();
      if (!box) return null;
      return {
        x: Math.round(box.left),
        y: Math.round(box.top),
        width: Math.round(box.width),
        height: Math.round(box.height)
      };
    } catch {
      return null;
    }
  }

  function refFor(el, store) {
    if (!store) {
      // Without a ref store no element can be named in a follow-up
      // action; that is an omission the snapshot must own up to,
      // never a silent one.
      state.truncated = true;
      return null;
    }
    try {
      const existing = store.map.get(el);
      if (existing) return existing;
      store.counter += 1;
      const ref = 'e' + String(store.counter) + '-' + store.scope;
      store.map.set(el, ref);
      store.reverse.set(ref, new WeakRef(el));
      return ref;
    } catch {
      state.truncated = true;
      return null;
    }
  }

  function serializeSvg(el) {
    const node = { role: 'image' };
    try {
      const title = el.getAttribute('aria-label') || el.getAttribute('title');
      if (title) node.name = clip(title, NAME_LIMIT);
    } catch {}
    node.rect = rectOf(el);
    return node;
  }

  function serializeElement(el, depth, store) {
    if (state.count >= MAX_NODES || depth >= MAX_DEPTH) {
      state.truncated = true;
      return null;
    }
    state.count += 1;

    const role = roleOf(el);
    const node = { role: role };
    const name = nameOf(el, role);
    if (name) node.name = name;
    const value = valueTextOf(el);
    if (value !== null) node.value = value;
    const checked = checkedOf(el, role);
    if (checked !== null) node.checked = checked;
    if (disabledOf(el)) node.disabled = true;
    const rect = rectOf(el);
    if (rect) node.rect = rect;
    if (ACTIONABLE_ROLES.has(role) && !node.disabled) {
      const ref = refFor(el, store);
      if (ref) node.ref = ref;
    }

    node.children = serializeChildren(el, depth, store);
    return node;
  }

  // `children`, `rows`, and `cells` are HTMLCollections: array-like and
  // live, but not iterable (only NodeList declares `iterable<Node>`),
  // so a walker copies one before reading it. The copy also freezes
  // the walk against a page that mutates the tree mid-snapshot.
  function arrayOf(collection) {
    return Array.prototype.slice.call(collection || []);
  }

  function pushChild(element, depth, store, out) {
    if (!element) return;
    if (SKIPPED_TAGS.has(element.tagName)) return;
    if (element.tagName === 'SVG') {
      const svg = serializeSvg(element);
      if (svg) out.push(svg);
      return;
    }
    if (!isVisible(element)) return;
    const serialized = serializeElement(element, depth + 1, store);
    if (serialized) out.push(serialized);
  }

  function pushAll(list, depth, store, out) {
    for (const element of arrayOf(list)) {
      pushChild(element, depth, store, out);
    }
  }

  function serializeChildren(el, depth, store) {
    const out = [];
    let shadow = null;
    try {
      shadow = el.shadowRoot;
    } catch {
      state.truncated = true;
    }

    // A host with an open shadow root reports only its shadow tree:
    // slotted light-DOM nodes appear inside their slot, and unrendered
    // light children are omitted (invisible means omitted, never
    // guessed at).
    if (shadow) {
      if (shadow.children) pushAll(shadow.children, depth, store, out);
      return out;
    }

    if (el.tagName === 'SLOT') {
      try {
        if (typeof el.assignedNodes === 'function') {
          const assigned = el.assignedNodes({ flatten: true });
          for (const node of assigned) {
            if (node && node.nodeType === 1) {
              pushChild(node, depth, store, out);
            }
          }
        }
      } catch {
        state.truncated = true;
      }
      return out;
    }

    try {
      pushAll(el.children, depth, store, out);
    } catch {
      state.truncated = true;
    }
    return out;
  }

  const store = ensureRefStore();
  const root = document.body || document.documentElement;
  const envelope = {
    version: 1,
    truncated: state.truncated,
    viewport: {
      width: window.innerWidth || 0,
      height: window.innerHeight || 0
    },
    scroll: { x: window.scrollX || 0, y: window.scrollY || 0 },
    root: { role: 'root', children: [] }
  };

  if (root) {
    const tree = serializeElement(root, 0, store);
    if (tree) envelope.root = tree;
  }
  envelope.truncated = state.truncated;
  return envelope;
})();
