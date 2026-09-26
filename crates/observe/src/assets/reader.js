/**
 * @fileoverview In-page reader for rutter markdown readouts.
 *
 * Walks the composed DOM (including open shadow roots and slot
 * assignments) and extracts the readable content as a Markdown
 * document, returned as a JSON envelope
 * `{ version, truncated, title, markdown }`. Site chrome (nav, aside,
 * footer, landmark roles), hidden elements, and non-content tags are
 * omitted. The script never throws: every guard degrades to
 * `truncated: true` with partial output, never an exception.
 *
 * The caller evaluates this script inside a page and converts the
 * returned envelope with rutter-observe. Owned by rutter-observe;
 * engine and orchestration code must not modify it.
 */
(function () {
  'use strict';

  var VERSION = 1;
  var MAX_NODES = 50000;
  var MAX_DEPTH = 200;
  var MAX_CHARS = 100000;
  var MAX_INLINE_CHARS = 20000;
  var TITLE_LIMIT = 200;

  // Non-content elements: never rendered. Form controls and dialogs
  // are interaction surfaces the accessibility snapshot already
  // covers; the readout is for readable content.
  var SKIPPED_TAGS = {
    SCRIPT: 1, STYLE: 1, NOSCRIPT: 1, TEMPLATE: 1, HEAD: 1,
    META: 1, LINK: 1, TITLE: 1, BASE: 1, DATALIST: 1,
    SVG: 1, IFRAME: 1, CANVAS: 1, DIALOG: 1,
    INPUT: 1, TEXTAREA: 1, SELECT: 1, OPTION: 1, BUTTON: 1
  };

  // Site chrome: content that frames the page rather than belonging
  // to it, by tag or landmark role.
  var CHROME_TAGS = { NAV: 1, ASIDE: 1, FOOTER: 1 };
  var CHROME_ROLES = {
    navigation: 1, complementary: 1, banner: 1, contentinfo: 1
  };

  var state = { nodes: 0, size: 0, truncated: false, blocks: [] };

  function clip(text, limit) {
    if (text.length <= limit) return text;
    var sliced = text.slice(0, limit - 3);
    // Never cut between a surrogate pair: a lone surrogate breaks
    // JSON consumers. Drop the orphaned high surrogate if present.
    var last = sliced.charCodeAt(sliced.length - 1);
    if (last >= 0xd800 && last <= 0xdbff) sliced = sliced.slice(0, -1);
    return sliced + '...';
  }

  function collapse(text) {
    return String(text || '').replace(/\s+/g, ' ').trim();
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

  function outOfBudget() {
    return state.nodes >= MAX_NODES || state.size >= MAX_CHARS;
  }

  function attribute(el, name) {
    try {
      return el.getAttribute(name);
    } catch (err) {
      return null;
    }
  }

  function absoluteUrl(url) {
    try {
      return new URL(url, document.baseURI).href;
    } catch (err) {
      return null;
    }
  }

  function tagName(el) {
    try {
      return el.tagName;
    } catch (err) {
      return '';
    }
  }

  function isChrome(el) {
    if (CHROME_TAGS[tagName(el)]) return true;
    var role = attribute(el, 'role');
    if (role) {
      var first = String(role).trim().split(/\s+/)[0];
      if (CHROME_ROLES[first]) return true;
    }
    return attribute(el, 'aria-hidden') === 'true';
  }

  function shouldSkip(el) {
    if (SKIPPED_TAGS[tagName(el)]) return true;
    if (isChrome(el)) return true;
    return !isVisible(el);
  }

  // Inline content of `el` as markdown: text plus links, images, and
  // lightweight emphasis. Capped per call so one huge paragraph
  // cannot balloon the output; the cap sets `truncated`.
  function inlineOf(el) {
    var out = inlineNodes(el.childNodes, 0);
    return collapse(out);
  }

  function inlineNodes(list, depth) {
    var out = '';
    for (var i = 0; i < list.length; i += 1) {
      if (out.length >= MAX_INLINE_CHARS) {
        state.truncated = true;
        break;
      }
      var node = list[i];
      if (!node) continue;
      if (node.nodeType === 3) {
        out += String(node.nodeValue || '').replace(/\s+/g, ' ');
        continue;
      }
      if (node.nodeType !== 1 || depth >= MAX_DEPTH) continue;
      var tag = tagName(node);
      if (SKIPPED_TAGS[tag] || isChrome(node) || !isVisible(node)) continue;
      if (tag === 'BR') {
        out += ' ';
        continue;
      }
      if (tag === 'A' && attribute(node, 'href')) {
        var inner = collapse(inlineNodes(node.childNodes, depth + 1));
        if (!inner) continue;
        var href = absoluteUrl(attribute(node, 'href'));
        var label = inner.replace(/\[/g, '\\[').replace(/\]/g, '\\]');
        out += href ? '[' + label + '](' + href + ')' : label;
        continue;
      }
      if (tag === 'IMG') {
        var alt = collapse(attribute(node, 'alt') || '');
        var src = absoluteUrl(attribute(node, 'src') || '');
        if (alt && src) out += '![' + alt.replace(/\]/g, '\\]') + '](' + src + ')';
        continue;
      }
      if (tag === 'CODE' || tag === 'KBD' || tag === 'SAMP') {
        var code = collapse(node.textContent);
        out += code.indexOf('`') === -1 ? '`' + code + '`' : code;
        continue;
      }
      if (tag === 'STRONG' || tag === 'B') {
        out += '**' + collapse(inlineNodes(node.childNodes, depth + 1)) + '**';
        continue;
      }
      if (tag === 'EM' || tag === 'I') {
        out += '*' + collapse(inlineNodes(node.childNodes, depth + 1)) + '*';
        continue;
      }
      out += inlineNodes(node.childNodes, depth + 1);
    }
    return out;
  }

  function push(text) {
    if (!text) return;
    state.blocks.push(text);
    state.size += text.length;
    if (state.size >= MAX_CHARS) state.truncated = true;
  }

  function pushParagraph(text) {
    var collapsed = collapse(text);
    if (collapsed) push(collapsed);
  }

  // Renders `render` and prefixes every line it produced, so
  // blockquotes can wrap any inner shape (paragraphs, pre, tables).
  function prefixed(prefix, render) {
    var start = state.blocks.length;
    render();
    for (var i = start; i < state.blocks.length; i += 1) {
      var lines = state.blocks[i].split('\n');
      for (var l = 0; l < lines.length; l += 1) {
        lines[l] = lines[l] ? prefix + lines[l] : prefix.trimEnd();
      }
      state.blocks[i] = lines.join('\n');
    }
  }

  function escapeCell(text) {
    return collapse(text).replace(/\|/g, '\\|');
  }

  function headingLevel(el) {
    var tag = tagName(el);
    var level = parseInt(tag.charAt(1), 10);
    if (!(level >= 1 && level <= 6)) return 2;
    return level;
  }

  function fenceFor(text) {
    var longest = 0;
    var runs = text.match(/`+/g) || [];
    for (var i = 0; i < runs.length; i += 1) {
      if (runs[i].length > longest) longest = runs[i].length;
    }
    return '`'.repeat(Math.max(3, longest + 1));
  }

  function emitCode(el) {
    var text = String(el.textContent || '').replace(/\n{3,}/g, '\n\n');
    var fence = fenceFor(text);
    push(fence + '\n' + text.replace(/\n+$/, '') + '\n' + fence);
  }

  function emitList(el, indent, depth) {
    var ordered = tagName(el) === 'OL';
    var index = 1;
    var items = el.children || [];
    for (var i = 0; i < items.length; i += 1) {
      if (state.truncated || outOfBudget()) return;
      var li = items[i];
      if (!li || tagName(li) !== 'LI') continue;
      if (!isVisible(li)) continue;
      var marker = ordered ? index + '. ' : '- ';
      index += 1;
      push(indent + marker + inlinePartOfItem(li, depth));
      emitNestedLists(li, indent + '  ', depth);
    }
  }

  // Inline content of one list item: its inline children plus any
  // nested block shapes except the nested lists, which the caller
  // emits separately with indentation.
  function inlinePartOfItem(li, depth) {
    var out = '';
    var parts = li.childNodes || [];
    for (var i = 0; i < parts.length; i += 1) {
      var node = parts[i];
      if (!node) continue;
      if (node.nodeType === 3) {
        out += String(node.nodeValue || '').replace(/\s+/g, ' ');
        continue;
      }
      if (node.nodeType !== 1) continue;
      var tag = tagName(node);
      if (tag === 'UL' || tag === 'OL') continue;
      out += ' ' + inlineOf(node);
    }
    return collapse(out);
  }

  function emitNestedLists(li, indent, depth) {
    var lists = li.children || [];
    for (var i = 0; i < lists.length; i += 1) {
      var tag = tagName(lists[i]);
      if (tag === 'UL' || tag === 'OL') {
        emitList(lists[i], indent, depth + 1);
      }
    }
  }

  function emitTable(el) {
    var rows = [];
    try {
      rows = Array.prototype.slice.call(el.querySelectorAll('tr'));
    } catch (err) {
      return;
    }
    if (rows.length === 0) return;
    var header = rowCells(rows[0]);
    if (header.length === 0) return;
    push('| ' + header.join(' | ') + ' |');
    var separator = [];
    for (var c = 0; c < header.length; c += 1) separator.push('---');
    push('| ' + separator.join(' | ') + ' |');
    for (var r = 1; r < rows.length; r += 1) {
      var cells = rowCells(rows[r]);
      while (cells.length < header.length) cells.push('');
      push('| ' + cells.slice(0, header.length).join(' | ') + ' |');
    }
  }

  function rowCells(row) {
    var cells = [];
    try {
      var parts = row.cells || [];
      for (var i = 0; i < parts.length; i += 1) {
        cells.push(escapeCell(inlineOf(parts[i])));
      }
    } catch (err) {}
    return cells;
  }

  function childNodesOf(el) {
    try {
      var shadow = el.shadowRoot;
      // A host with an open shadow root contributes only its shadow
      // tree, mirroring the serializer's traversal.
      if (shadow) return shadow.childNodes || [];
      if (tagName(el) === 'SLOT' && typeof el.assignedNodes === 'function') {
        return el.assignedNodes({ flatten: true }) || [];
      }
      return el.childNodes || [];
    } catch (err) {
      state.truncated = true;
      return [];
    }
  }

  function emitBlock(el, depth) {
    if (depth >= MAX_DEPTH || outOfBudget()) {
      state.truncated = true;
      return;
    }
    state.nodes += 1;
    if (shouldSkip(el)) return;

    var tag = tagName(el);
    if (/^H[1-6]$/.test(tag)) {
      var level = headingLevel(el);
      var text = inlineOf(el);
      if (text) push('#'.repeat(level) + ' ' + text);
      return;
    }
    if (tag === 'P') {
      pushParagraph(inlineOf(el));
      return;
    }
    if (tag === 'UL' || tag === 'OL') {
      emitList(el, '', depth);
      return;
    }
    if (tag === 'TABLE') {
      emitTable(el);
      return;
    }
    if (tag === 'PRE') {
      emitCode(el);
      return;
    }
    if (tag === 'BLOCKQUOTE') {
      prefixed('> ', function () { emitChildren(el, depth); });
      return;
    }
    if (tag === 'HR') {
      push('---');
      return;
    }
    if (tag === 'IMG') {
      var src = absoluteUrl(attribute(el, 'src') || '');
      if (src) {
        var alt = collapse(attribute(el, 'alt') || '');
        push('![' + alt.replace(/\]/g, '\\]') + '](' + src + ')');
      }
      return;
    }
    if (tag === 'A' && attribute(el, 'href')) {
      pushParagraph(inlineOf(el));
      return;
    }
    if (tag === 'DT' || tag === 'DD') {
      pushParagraph(inlineOf(el));
      return;
    }
    emitChildren(el, depth);
  }

  function emitChildren(el, depth) {
    var nodes = childNodesOf(el);
    for (var i = 0; i < nodes.length; i += 1) {
      if (state.truncated) return;
      var node = nodes[i];
      if (!node) continue;
      if (node.nodeType === 3) {
        pushParagraph(String(node.nodeValue || ''));
        continue;
      }
      if (node.nodeType === 1) emitBlock(node, depth + 1);
    }
  }

  function joinBlocks() {
    var body = state.blocks.join('\n\n');
    var cleaned = body.replace(/\n{3,}/g, '\n\n').replace(/^\n+/, '');
    if (cleaned.length > MAX_CHARS) return clip(cleaned, MAX_CHARS);
    return cleaned;
  }

  var title = '';
  try {
    title = clip(collapse(document.title), TITLE_LIMIT);
  } catch (err) {
    state.truncated = true;
  }

  var envelope = {
    version: VERSION,
    truncated: state.truncated,
    title: title,
    markdown: ''
  };

  try {
    var root = document.body || document.documentElement;
    if (root) emitBlock(root, 0);
    envelope.markdown = joinBlocks();
  } catch (err) {
    state.truncated = true;
    envelope.markdown = joinBlocks();
  }
  envelope.truncated = state.truncated;
  return envelope;
})();
