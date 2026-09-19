/**
 * @fileoverview rutter dashboard client: connects to the event stream,
 * renders the action timeline and pending approvals, and submits human
 * decisions. No build step; vanilla ES2017+ only. The access token
 * arrives in the first visit's query and is exchanged for an HttpOnly
 * cookie by the server; this script never persists it.
 */
(function () {
  'use strict';
  var I18N = {};
  // First visit carries the token in the query; the server exchanges it
  // for an HttpOnly session cookie (blueprint 7.7), so page scripts
  // never store or read it. Requests below fall back to the cookie
  // when no query token is present (a refresh, a bookmarked path).
  var token = new URLSearchParams(window.location.search).get('token');
  try { window.localStorage.removeItem('rutterToken'); } catch (error) {}

  function withToken(path) {
    return token ? path + '?token=' + encodeURIComponent(token) : path;
  }

  fetch(withToken('/i18n/en.json'))
    .then(function (response) { return response.json(); })
    .then(function (catalog) {
      I18N = catalog;
      document.querySelectorAll('[data-i18n]').forEach(function (node) {
        node.textContent = I18N[node.getAttribute('data-i18n')] || node.getAttribute('data-i18n');
      });
      connect();
    })
    .catch(function () {
      // The catalog is cosmetic; without it the dashboard still works
      // with untranslated labels instead of never connecting.
      connect();
    });

  function t(key) { return I18N[key] || key; }

  var socket = null;
  // Seconds between reconnect attempts; reset on a successful open and
  // capped so a dead server cannot spin the loop forever.
  var reconnectDelay = 1000;

  function connect() {
    // The server replays every session's history on connect, so a
    // reconnect must start from an empty timeline or the replay appends
    // a second copy of every event.
    var timeline = document.getElementById('events');
    if (timeline) { timeline.textContent = ''; }
    var protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    socket = new WebSocket(
      protocol + '//' + window.location.host + withToken('/ws'));
    socket.binaryType = 'arraybuffer';

    socket.onmessage = function (message) {
      if (message.data instanceof ArrayBuffer) {
        showFrame(message.data);
        return;
      }
      var envelope;
      try { envelope = JSON.parse(message.data); } catch (error) { return; }
      if (envelope.type === 'note') { append(envelope.text); return; }
      if (envelope.type === 'decision-ack' || envelope.type === 'screencast-ack') { return; }
      render(envelope);
    };
    socket.onopen = function () { reconnectDelay = 1000; };
    socket.onclose = function () {
      append(t('reconnecting'));
      setTimeout(connect, reconnectDelay);
      reconnectDelay = Math.min(reconnectDelay * 2, 30000);
    };
  }

  function sendScreencast(on) {
    if (!socket || socket.readyState !== 1) { return; }
    var session = document.getElementById('live-session').value.trim();
    if (on && !session) { return; }
    socket.send(JSON.stringify({ type: 'screencast', on: on, session: session }));
  }

  function showFrame(buffer) {
    var image = document.getElementById('live-frame');
    image.hidden = false;
    if (image.src.startsWith('blob:')) { URL.revokeObjectURL(image.src); }
    image.src = URL.createObjectURL(new Blob([buffer], { type: 'image/jpeg' }));
  }

  // The script loads at the end of <body>, so the controls exist now.
  document.getElementById('live-start').onclick = function () {
    sendScreencast(true);
  };
  document.getElementById('live-stop').onclick = function () {
    sendScreencast(false);
  };

  function render(envelope) {
    var event = envelope.event || {};
    switch (event.type) {
      case 'session_started':
      case 'session_closed':
        refreshSessions();
        break;
      case 'approval_requested':
        showApproval(envelope.session, event);
        break;
      case 'approval_resolved':
        hideApproval(event.request_id);
        break;
      default:
        append(describe(envelope));
    }
  }

  function describe(envelope) {
    var event = envelope.event || {};
    var base = '#' + envelope.seq + ' ' + envelope.recorded_at + ' ' +
      envelope.session + ' ' + event.type;
    if (event.action && event.action.type) {
      base += ' ' + event.action.type;
    }
    if (event.error) {
      base += ' error=' + event.error.type;
    }
    return base;
  }

  function showApproval(session, event) {
    // Reconnect replays re-deliver requests already on screen; drop the
    // stale card first or duplicate ids pile up on the approval list.
    hideApproval(event.request_id);
    var node = document.createElement('div');
    node.className = 'approval';
    node.id = event.request_id;
    node.textContent = '[' + event.request_id + '] ' + session + ': ' +
      (event.action ? event.action.type : 'action');
    var grant = document.createElement('button');
    grant.textContent = t('grant');
    grant.onclick = function () { decide(event.request_id, true); };
    var deny = document.createElement('button');
    deny.textContent = t('deny');
    deny.onclick = function () { decide(event.request_id, false); };
    node.appendChild(grant);
    node.appendChild(deny);
    document.getElementById('approval-list').appendChild(node);
  }

  function hideApproval(requestId) {
    var node = document.getElementById(requestId);
    if (node) { node.remove(); }
  }

  function decide(requestId, grant) {
    fetch(withToken('/api/decisions'), {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ request_id: requestId, grant: grant })
    });
  }

  function refreshSessions() {
    document.getElementById('sessions').textContent = '';
  }

  // Every line (events, notes, connection status) lands in the single
  // timeline content container (`#events`), so a reconnect can clear
  // them all together.
  function append(text) {
    var node = document.createElement('div');
    node.className = 'event';
    node.textContent = text;
    var list = document.getElementById('events');
    if (list) { list.prepend(node); }
  }
})();
