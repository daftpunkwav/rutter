/**
 * @fileoverview In-page DOM serializer for rutter accessibility snapshots.
 *
 * Walks the composed DOM (including open shadow roots and slot
 * assignments) and reports a plain JSON tree per docs/SNAPSHOT_SPEC.md
 * sections 2-4. Hidden elements are omitted, never guessed at. The
 * script never throws: every per-node computation is guarded and a
 * failure degrades that node to role "generic".
 *
 * The caller evaluates this script inside a page and converts the
 * returned envelope with rutter-observe. Owned by rutter-observe;
 * engine and orchestration code must not modify it.
 */
(function () {
  'use strict';

  var MAX_NODES = 50000;
  var MAX_DEPTH = 200;
  var NAME_LIMIT = 120;
  var VALUE_LIMIT = 200;

  var SKIPPED_TAGS = {
    SCRIPT: 1, STYLE: 1, NOSCRIPT: 1, TEMPLATE: 1, HEAD: 1,
    META: 1, LINK: 1, TITLE: 1, BR: 1, BASE: 1, DATALIST: 1
  };

  var IMPLICIT_ROLES = {
    A: 'link', AREA: 'link', BUTTON: 'button', SELECT: 'combobox',
    OPTION: 'option', OPTGROUP: 'group', TEXTAREA: 'textbox',
    H1: 'heading', H2: 'heading', H3: 'heading', H4: 'heading',
    H5: 'heading', H6: 'heading', IMG: 'image', NAV: 'navigation',
    UL: 'list', OL: 'list', LI: 'listitem', TABLE: 'table',
    TR: 'row', TD: 'cell', TH: 'columnheader', FORM: 'form',
    MAIN: 'main', HEADER: 'banner', FOOTER: 'contentinfo',
    ASIDE: 'complementary', DIALOG: 'dialog', PROGRESS: 'progressbar',
    DETAILS: 'group', SUMMARY: 'button', HR: 'separator',
    OUTPUT: 'status', METER: 'meter'
  };

  var ACTIONABLE_ROLES = {
    button: 1, link: 1, textbox: 1, searchbox: 1, checkbox: 1,
    radio: 1, combobox: 1, listbox: 1, option: 1, menuitem: 1,
    tab: 1, slider: 1, spinbutton: 1, switch: 1, treeitem: 1
  };

  // Roles whose accessible name may come from their text content.
  // Containers (generic, list, group, ...) must not inherit the page
  // text: their names stay empty like the ARIA spec requires.
  var TEXT_NAME_ROLES = {
    heading: 1, button: 1, link: 1, listitem: 1, option: 1,
    cell: 1, columnheader: 1, rowheader: 1, menuitem: 1, tab: 1,
    treeitem: 1, term: 1, definition: 1, alert: 1, status: 1,
    paragraph: 1, caption: 1, code: 1, emphasis: 1, strong: 1,
    time: 1, blockquote: 1, note: 1, tooltip: 1
  };

  var state = { count: 0, truncated: false };

  function ensureRefStore() {
    try {
      if (!window.__rutterRefStore) {
        window.__rutterRefStore = { map: new WeakMap(), counter: 0 };
      }
      return window.__rutterRefStore;
    } catch (err) {
      state.truncated = true;
      return null;
    }
  }

  function inputRole(el) {
    var type = 'text';
    try {
      var explicit = el.getAttribute('type');
      if (explicit) type = explicit.toLowerCase();
    } catch (err) {}
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
      var explicit = el.getAttribute('role');
      if (explicit) {
        var first = explicit.trim().split(/\s+/)[0];
        if (first) return first;
      }
    } catch (err) {}
    if (el.tagName === 'INPUT') return inputRole(el);
    var implicit = IMPLICIT_ROLES[el.tagName];
    return implicit || 'generic';
  }

  function isVisible(el) {
    try {
      var rects = el.getClientRects();
      if (!rects || rects.length === 0) return false;
      var style = window.getComputedStyle(el);
      return style.visibility !== 'hidden' && style.visibility !== 'collapse';
    } catch (err) {
      return false;
    }
  }

  function clip(text, limit) {
    if (text.length <= limit) return text;
    var sliced = text.slice(0, limit - 3);
    // Never cut between a surrogate pair: a lone surrogate breaks
    // JSON consumers. Drop the orphaned high surrogate if present.
    var last = sliced.charCodeAt(sliced.length - 1);
    if (last >= 0xd800 && last <= 0xdbff) sliced = sliced.slice(0, -1);
    return sliced + '...';
  }

  function nameOf(el, role) {
    try {
      var label = el.getAttribute('aria-label');
      if (label && label.trim()) return clip(label.replace(/\s+/g, ' ').trim(), NAME_LIMIT);
      if (role === 'image') {
        var alt = el.getAttribute('alt');
        if (alt && alt.trim()) return clip(alt.replace(/\s+/g, ' ').trim(), NAME_LIMIT);
      }
      if (el.labels && el.labels.length > 0) {
        var labelText = (el.labels[0].textContent || '').replace(/\s+/g, ' ').trim();
        if (labelText) return clip(labelText, NAME_LIMIT);
      }
      var placeholder = el.getAttribute('placeholder');
      if (placeholder && placeholder.trim()) return clip(placeholder, NAME_LIMIT);
      var title = el.getAttribute('title');
      if (title && title.trim()) return clip(title, NAME_LIMIT);
      if (!TEXT_NAME_ROLES[role]) return null;
      var text = (el.textContent || '').replace(/\s+/g, ' ').trim();
      return text ? clip(text, NAME_LIMIT) : null;
    } catch (err) {
      return null;
    }
  }

  function valueOf(el) {
    try {
      var tag = el.tagName;
      if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') {
        var value = String(el.value || '');
        return value ? clip(value, VALUE_LIMIT) : null;
      }
    } catch (err) {}
    return null;
  }

  function checkedOf(el) {
    try {
      var role = roleOf(el);
      if (role === 'checkbox' || role === 'radio') return Boolean(el.checked);
    } catch (err) {}
    return null;
  }

  function disabledOf(el) {
    try {
      if (el.disabled) return true;
      var aria = el.getAttribute('aria-disabled');
      return aria === 'true';
    } catch (err) {
      return false;
    }
  }

  function rectOf(el) {
    try {
      var box = el.getBoundingClientRect();
      if (!box) return null;
      return {
        x: Math.round(box.left),
        y: Math.round(box.top),
        width: Math.round(box.width),
        height: Math.round(box.height)
      };
    } catch (err) {
      return null;
    }
  }

  function refFor(el, store) {
    if (!store) return null;
    try {
      var existing = store.map.get(el);
      if (existing) return existing;
      store.counter += 1;
      var ref = 'e' + String(store.counter);
      store.map.set(el, ref);
      return ref;
    } catch (err) {
      return null;
    }
  }

  function serializeSvg(el) {
    var node = { role: 'image' };
    try {
      var title = el.getAttribute('aria-label') || el.getAttribute('title');
      if (title) node.name = clip(title, NAME_LIMIT);
    } catch (err) {}
    node.rect = rectOf(el);
    return node;
  }

  function serializeElement(el, depth, store) {
    if (state.count >= MAX_NODES || depth >= MAX_DEPTH) {
      state.truncated = true;
      return null;
    }
    state.count += 1;

    var role = roleOf(el);
    var node = { role: role };
    var name = nameOf(el, role);
    if (name) node.name = name;
    var value = valueOf(el);
    if (value !== null) node.value = value;
    var checked = checkedOf(el);
    if (checked !== null) node.checked = checked;
    if (disabledOf(el)) node.disabled = true;
    var rect = rectOf(el);
    if (rect) node.rect = rect;
    if (ACTIONABLE_ROLES[role] && !node.disabled) {
      var ref = refFor(el, store);
      if (ref) node.ref = ref;
    }

    node.children = serializeChildren(el, depth, store);
    return node;
  }

  function pushChild(element, depth, store, out) {
    if (!element) return;
    if (SKIPPED_TAGS[element.tagName]) return;
    if (element.tagName === 'SVG') {
      var svg = serializeSvg(element);
      if (svg) out.push(svg);
      return;
    }
    if (!isVisible(element)) return;
    var serialized = serializeElement(element, depth + 1, store);
    if (serialized) out.push(serialized);
  }

  function pushAll(list, depth, store, out) {
    for (var i = 0; i < list.length; i += 1) {
      pushChild(list[i], depth, store, out);
    }
  }

  function serializeChildren(el, depth, store) {
    var out = [];
    var shadow = null;
    try {
      shadow = el.shadowRoot;
    } catch (err) {
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
          var assigned = el.assignedNodes({ flatten: true });
          for (var a = 0; a < assigned.length; a += 1) {
            if (assigned[a] && assigned[a].nodeType === 1) {
              pushChild(assigned[a], depth, store, out);
            }
          }
        }
      } catch (err) {
        state.truncated = true;
      }
      return out;
    }

    try {
      pushAll(el.children, depth, store, out);
    } catch (err) {
      state.truncated = true;
    }
    return out;
  }

  var store = ensureRefStore();
  var root = document.body || document.documentElement;
  var envelope = {
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
    var tree = serializeElement(root, 0, store);
    if (tree) envelope.root = tree;
  }
  envelope.truncated = state.truncated;
  return envelope;
})();
