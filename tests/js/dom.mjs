/**
 * @fileoverview Minimal in-memory DOM for the page scripts' tests.
 *
 * Boundary: enough of the platform for the injected page scripts
 * (the serializer, the reader, and the dashboard client) to run
 * unmodified under `node --test`. It is a test double, not a browser:
 * it implements exactly the surface those three files touch, and
 * nothing else. Anything a script starts using that this file does
 * not model shows up as a `TypeError` in the test that needs it,
 * which is the point -- a silently missing member would make a
 * behavioural test vacuous.
 *
 * No dependencies: `node --test` and `node:vm` are Node builtins.
 */

import vm from 'node:vm';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));

/** Reads one of the repository's shipped JavaScript files. */
export function loadScript(relativePath) {
  return readFileSync(join(here, '..', '..', relativePath), 'utf8');
}

let nextNodeId = 1;

class TextNode {
  constructor(value) {
    this.nodeType = 3;
    this.nodeValue = value;
    this.parent = null;
  }

  get textContent() {
    return this.nodeValue;
  }
}

class Element {
  constructor(tagName) {
    this.nodeType = 1;
    this.tagName = String(tagName).toUpperCase();
    this.attributes = Object.create(null);
    this.childNodes = [];
    this.parent = null;
    this.shadowRoot = null;
    this.assigned = null;
    this._id = nextNodeId++;
    this.style = {};
    // Layout surface: an element is rendered until told otherwise.
    this.layout = { rects: true, box: null, visibility: 'visible', display: '' };
  }

  // --- tree -----------------------------------------------------------

  get children() {
    return this.childNodes.filter((node) => node.nodeType === 1);
  }

  get textContent() {
    return this.childNodes.map((node) => node.textContent).join('');
  }

  // Assigning text replaces the children, as in the DOM.
  set textContent(value) {
    this.childNodes = [];
    if (value !== '' && value !== null && value !== undefined) {
      this.appendChild(new TextNode(String(value)));
    }
  }

  /** Detaches this node from its parent, as `ChildNode.remove` does. */
  remove() {
    if (!this.parent) return;
    const at = this.parent.childNodes.indexOf(this);
    if (at >= 0) this.parent.childNodes.splice(at, 1);
    this.parent = null;
  }

  appendChild(node) {
    node.parent = this;
    this.childNodes.push(node);
    return node;
  }

  append(...nodes) {
    nodes.forEach((node) => this.appendChild(node));
    return this;
  }

  prepend(node) {
    node.parent = this;
    this.childNodes.unshift(node);
    return node;
  }

  get lastChild() {
    return this.childNodes[this.childNodes.length - 1] || null;
  }

  removeChild(node) {
    const at = this.childNodes.indexOf(node);
    if (at >= 0) this.childNodes.splice(at, 1);
    node.parent = null;
    return node;
  }

  // --- attributes -----------------------------------------------------

  getAttribute(name) {
    const value = this.attributes[name];
    return value === undefined ? null : value;
  }

  setAttribute(name, value) {
    this.attributes[name] = String(value);
  }

  hasAttribute(name) {
    return this.attributes[name] !== undefined;
  }

  get id() {
    return this.getAttribute('id') || '';
  }

  set id(value) {
    this.setAttribute('id', value);
  }

  get className() {
    return this.getAttribute('class') || '';
  }

  set className(value) {
    this.setAttribute('class', value);
  }

  get hidden() {
    return this.hasAttribute('hidden');
  }

  set hidden(value) {
    if (value) this.setAttribute('hidden', '');
    else delete this.attributes.hidden;
  }

  // --- form / value surface -------------------------------------------

  get value() {
    return this._value === undefined ? '' : this._value;
  }

  set value(next) {
    this._value = next;
  }

  get checked() {
    return Boolean(this._checked);
  }

  get disabled() {
    return Boolean(this._disabled);
  }

  get labels() {
    return this._labels || [];
  }

  // --- layout surface -------------------------------------------------

  getClientRects() {
    return this.layout.rects ? [{}] : [];
  }

  getBoundingClientRect() {
    return this.layout.box || { left: 0, top: 0, width: 1, height: 1 };
  }

  // --- table surface --------------------------------------------------

  // The table's own rows: a table nested inside a cell contributes its
  // rows to itself instead of interleaving them into the outer table.
  get rows() {
    const out = [];
    const walk = (node) => {
      for (const child of node.children) {
        if (child.tagName === 'TABLE') continue;
        if (child.tagName === 'TR') out.push(child);
        else walk(child);
      }
    };
    walk(this);
    return out;
  }

  get cells() {
    return this.children.filter((cell) => cell.tagName === 'TD' || cell.tagName === 'TH');
  }

  /** A slot's flattened assignment, or nothing when it has none. */
  assignedNodes() {
    return this.assigned || [];
  }
}

/**
 * Builds a tree from a compact literal.
 *
 *   el('div', { attrs: { role: 'main' } }, [ 'text', el('p', {}, ['hi']) ])
 *
 * A string child becomes a text node, an element child is appended
 * recursively, and `opts` may carry `hidden`, `value`, `checked`,
 * `disabled`, `display`, `rects`, `shadow` (replaces the light tree)
 * and `assigned` (a slot's flattened assignment).
 */
export function el(tag, opts = {}, children = []) {
  const node = new Element(tag);
  Object.entries(opts.attrs || {}).forEach(([name, value]) => node.setAttribute(name, value));
  if (opts.hidden) node.hidden = true;
  if (opts.value !== undefined) node.value = opts.value;
  if (opts.checked) node._checked = true;
  if (opts.disabled) node._disabled = true;
  if (opts.display !== undefined) node.layout.display = opts.display;
  if (opts.rects === false) node.layout.rects = false;
  for (const child of [].concat(children)) {
    node.appendChild(typeof child === 'string' ? new TextNode(child) : child);
  }
  if (opts.shadow) node.shadowRoot = build(opts.shadow);
  if (opts.assigned) {
    node.assigned = [].concat(opts.assigned).map((child) =>
      typeof child === 'string' ? new TextNode(child) : child
    );
  }
  if (opts.refs) node.labels = [].concat(opts.refs);
  return node;
}

function build(node) {
  return node;
}

/** A visible element with the given text content. */
export function text(tag, content, opts = {}) {
  return el(tag, opts, [content]);
}

/** An element that is laid out but reports `display: none` visibility-wise. */
export function invisible(tag, opts = {}, children = []) {
  return el(tag, { ...opts, rects: false }, children);
}

/**
 * A document whose body is `body` (a plain `<body>` when omitted).
 * `ids` are registered on the element registry the dashboard client
 * reads through `getElementById`. `noBody` models a document the
 * script has to fall back to `documentElement` for.
 */
export function makeDocument({
  title = 'Fixture',
  baseURI = 'https://page.example/a/b',
  body = null,
  noBody = false,
  ids = {},
} = {}) {
  const registry = new Map(Object.entries(ids));
  const bodyNode = noBody ? null : body || el('body');
  const documentElement = el('html', {}, bodyNode ? [bodyNode] : []);
  const document = {
    title,
    baseURI,
    body: bodyNode,
    documentElement,
    createElement(tag) {
      const node = new Element(tag);
      const originalSet = node.setAttribute.bind(node);
      node.setAttribute = (name, value) => {
        originalSet(name, value);
        if (name === 'id' && value) registry.set(String(value), node);
      };
      return node;
    },
    getElementById(id) {
      return registry.get(id) || null;
    },
    querySelectorAll(selector) {
      if (selector !== '[data-i18n]') return [];
      const found = [];
      const walk = (node) => {
        for (const child of node.children) {
          if (child.hasAttribute('data-i18n')) found.push(child);
          walk(child);
        }
      };
      walk(documentElement);
      return {
        forEach(callback) {
          found.forEach(callback);
        },
        get length() {
          return found.length;
        },
      };
    },
  };
  return { document, registry, bodyNode };
}

/** `getComputedStyle` over the fixture's per-element layout record. */
function getComputedStyle(node) {
  const style = node && node.layout ? node.layout : { visibility: 'visible', display: '' };
  return {
    visibility: style.visibility,
    display: style.display,
  };
}

/** A `WeakRef` whose collected state the test drives by hand. */
export class ControllableWeakRef {
  constructor(target) {
    this.target = target;
    this.collected = false;
  }

  deref() {
    return this.collected ? undefined : this.target;
  }
}

/**
 * A page: a sandbox the script runs in, owned by the caller.
 *
 * `page.window` is the live global object, so a test can read back
 * what the script left on it (`__rutterRefStore`, its own globals)
 * and can swap the GC primitive through `options.extra`.
 */
export function createPage({ document, window: windowExtras = {}, extra = {} } = {}) {
  const sandbox = {
    document,
    console,
    WeakMap,
    Map,
    Set,
    WeakRef: ControllableWeakRef,
    ArrayBuffer,
    Uint8Array,
    URL,
    URLSearchParams,
    Blob,
    JSON,
    Promise,
    setTimeout,
    clearTimeout,
    ...windowExtras,
    ...extra,
  };
  sandbox.window = sandbox;
  sandbox.globalThis = sandbox;
  sandbox.getComputedStyle = windowExtras.getComputedStyle || getComputedStyle;
  vm.createContext(sandbox);
  return {
    window: sandbox,
    document,
    run(source) {
      return vm.runInContext(source, sandbox, { filename: 'page-script.js' });
    },
  };
}
