/**
 * @fileoverview rutter dashboard client: connects to the event stream,
 * renders the action timeline and pending approvals, and submits human
 * decisions. No build step; vanilla ES2017+ only. The access token
 * arrives in the first visit's query and is exchanged for an HttpOnly
 * cookie by the server; this script never persists it.
 */
(() => {
  'use strict';
  // User-visible prose by key. A Map, not an object: the keys are read
  // off the document (`data-i18n`), and an object literal would answer
  // for `constructor` and the rest of `Object.prototype` too.
  let I18N = new Map();
  // First visit carries the token in the query; the server exchanges it
  // for an HttpOnly session cookie, so page scripts
  // never store or read it. Requests below fall back to the cookie
  // when no query token is present (a refresh, a bookmarked path).
  const token = new URLSearchParams(window.location.search).get('token');
  try { window.localStorage.removeItem('rutterToken'); } catch {}

  function withToken(path) {
    return token ? path + '?token=' + encodeURIComponent(token) : path;
  }

  fetch(withToken('/i18n/en.json'))
    .then((response) => response.json())
    .then((catalog) => {
      I18N = new Map(Object.entries(catalog));
      document.querySelectorAll('[data-i18n]').forEach((node) => {
        const key = node.getAttribute('data-i18n');
        node.textContent = t(key);
      });
      connect();
    })
    .catch(() => {
      // The catalog is cosmetic; without it the dashboard still works
      // with untranslated labels instead of never connecting.
      connect();
    });

  function t(key) { return I18N.get(key) || key; }

  let socket = null;
  // Sessions known from the event stream. connect() clears this on
  // every (re)connect and the replay rebuilds it; the live events keep
  // it current afterwards.
  let knownSessions = new Set();
  // Milliseconds between reconnect attempts; reset on a successful open
  // and capped so a dead server cannot spin the loop forever.
  let reconnectDelay = 1000;

  function connect() {
    // The server replays every session's history on connect, so a
    // reconnect must start from an empty timeline or the replay appends
    // a second copy of every event. The session set rebuilds the same
    // way — a session closed while the socket was down is absent from
    // the replay (its ring is dropped on close) and must not linger.
    const timeline = document.getElementById('events');
    if (timeline) { timeline.textContent = ''; }
    knownSessions = new Set();
    // The set is re-rendered, not just reset: a dashboard whose last
    // sessions all closed while the socket was down receives no
    // session event at all, so nothing would ever take the stale
    // chips (and the stale id options) off the screen.
    renderSessions();
    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    socket = new WebSocket(
      protocol + '//' + window.location.host + withToken('/ws'));
    socket.binaryType = 'arraybuffer';

    socket.onmessage = (message) => {
      if (message.data instanceof ArrayBuffer) {
        showFrame(message.data);
        return;
      }
      let envelope;
      try { envelope = JSON.parse(message.data); } catch { return; }
      if (envelope.type === 'note') { append(envelope.text); return; }
      if (envelope.type === 'decision-ack') { return; }
      if (envelope.type === 'screencast-ack') {
        // A refusal carries its reason;
        // silence would leave the operator a blank live view.
        if (envelope.reason) { append(t('screencastRefused') + ' ' + envelope.reason); }
        return;
      }
      if (envelope.type === 'screencast-stopped') {
        // The capture ended on the server side (a closed page, an
        // engine restart); without this the last frame would sit
        // there posing as a live one. Resuming is the viewer's call,
        // not an automatic reconnect.
        const frame = document.getElementById('live-frame');
        if (frame) {
          if (frame.src.startsWith('blob:')) { URL.revokeObjectURL(frame.src); }
          frame.hidden = true;
        }
        append(t('screencastStopped') + ' ' + envelope.reason);
        return;
      }
      render(envelope);
    };
    socket.onopen = () => { reconnectDelay = 1000; };
    socket.onclose = () => {
      append(t('reconnecting'));
      setTimeout(connect, reconnectDelay);
      reconnectDelay = Math.min(reconnectDelay * 2, 30000);
    };
  }

  function sendScreencast(on) {
    if (socket?.readyState !== 1) { return; }
    const session = document.getElementById('live-session').value.trim();
    if (on && !session) { return; }
    socket.send(JSON.stringify({ type: 'screencast', on: on, session: session }));
  }

  function showFrame(buffer) {
    const image = document.getElementById('live-frame');
    image.hidden = false;
    if (image.src.startsWith('blob:')) { URL.revokeObjectURL(image.src); }
    image.src = URL.createObjectURL(new Blob([buffer], { type: 'image/jpeg' }));
  }

  // The script loads at the end of <body>, so the controls exist now.
  document.getElementById('live-start').onclick = () => {
    sendScreencast(true);
  };
  document.getElementById('live-stop').onclick = () => {
    sendScreencast(false);
  };

  function render(envelope) {
    const event = envelope.event || {};
    switch (event.type) {
      case 'session_started':
      case 'session_closed':
        trackSession(envelope, event.type);
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

  function trackSession(envelope, type) {
    if (type === 'session_started') {
      knownSessions.add(envelope.session);
    } else {
      knownSessions.delete(envelope.session);
    }
    renderSessions();
  }

  function renderSessions() {
    const list = document.getElementById('sessions');
    const options = document.getElementById('session-ids');
    if (!list) { return; }
    list.textContent = '';
    if (options) { options.textContent = ''; }
    Array.from(knownSessions).sort().forEach((id) => {
      const item = document.createElement('span');
      item.className = 'session-chip';
      item.textContent = id + ' ';
      list.appendChild(item);
      if (options) {
        const option = document.createElement('option');
        option.value = id;
        options.appendChild(option);
      }
    });
  }

  function describe(envelope) {
    const event = envelope.event || {};
    let base = '#' + envelope.seq + ' ' + envelope.recorded_at + ' ' +
      envelope.session + ' ' + event.type;
    if (event.action?.type) {
      base += ' ' + event.action.type;
    }
    if (event.error) {
      base += ' error=' + event.error.type;
    }
    return base;
  }

  function effectText(effect) {
    // The brief names its own effect. An action shows its type and
    // element; a cookie write shows the write, because the action
    // vocabulary has no variant for it and a stand-in action would put
    // a different promise in front of the human than the one they keep.
    if (!effect?.kind) { return t('effectUnknown'); }
    if (effect.kind === 'cookies') {
      return t('effectCookies').replace('{count}', String(effect.count));
    }
    const action = effect.action || {};
    if (action.type === 'navigate') { return t('effectNavigate') + ' ' + (action.url || ''); }
    if (action.reference) { return t('effectOn') + ' ' + action.type + ' ' + action.reference; }
    return t('effectRun') + ' ' + (action.type || t('effectUnknown'));
  }

  function basisText(basis) {
    if (!basis?.kind) { return ''; }
    if (basis.kind === 'rule') {
      const rule = t('basisRule').replace('{index}', String(basis.index));
      return basis.url_pattern ? rule + ' (' + basis.url_pattern + ')' : rule;
    }
    if (basis.kind === 'missing_url') { return t('basisMissingUrl'); }
    return t('basisDefault');
  }

  function approvalLine(label, value) {
    const node = document.createElement('div');
    // Text nodes only: the judged URL and the reference both come from
    // page-controlled data, so this must never parse as markup.
    node.textContent = label + ': ' + value;
    return node;
  }

  function showApproval(session, event) {
    // Reconnect replays re-deliver requests already on screen; drop the
    // stale card first or duplicate ids pile up on the approval list.
    hideApproval(event.request_id);
    const brief = event.brief || {};
    const node = document.createElement('div');
    node.className = 'approval';
    node.id = event.request_id;

    const heading = document.createElement('div');
    heading.textContent = '[' + event.request_id + '] ' + session;
    node.appendChild(heading);
    node.appendChild(approvalLine(t('labelClass'), brief.class || '?'));
    node.appendChild(approvalLine(t('labelEffect'), effectText(brief.effect)));
    node.appendChild(
      approvalLine(t('labelTarget'), brief.judged_url || t('targetUnknown'))
    );
    node.appendChild(approvalLine(t('labelBasis'), basisText(brief.basis)));

    const grant = document.createElement('button');
    grant.textContent = t('grant');
    grant.onclick = () => { decide(event.request_id, true); };
    const deny = document.createElement('button');
    deny.textContent = t('deny');
    deny.onclick = () => { decide(event.request_id, false); };
    node.appendChild(grant);
    node.appendChild(deny);
    document.getElementById('approval-list').appendChild(node);
  }

  function hideApproval(requestId) {
    const node = document.getElementById(requestId);
    if (node) { node.remove(); }
  }

  // Milliseconds a decision post may take before it is abandoned. The
  // server answers in milliseconds when it is alive; a post that hangs
  // this long is a wedged server, and the operator must see the failure
  // instead of a card that sits there looking undecided.
  const DECIDE_TIMEOUT = 10000;

  function decide(requestId, grant) {
    // A decision that never lands leaves the agent parked until its
    // window closes, so a failed post is surfaced in the timeline
    // instead of looking like a grant that did nothing.
    const controller = new AbortController();
    const timer = setTimeout(() => { controller.abort(); }, DECIDE_TIMEOUT);
    fetch(withToken('/api/decisions'), {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ request_id: requestId, grant: grant }),
      signal: controller.signal
    })
      .then((response) => {
        clearTimeout(timer);
        if (!response.ok) {
          append(t('decisionRefused') + ' ' + requestId + ' (' + response.status + ')');
        }
      })
      .catch(() => {
        clearTimeout(timer);
        append(t('decisionUnreachable') + ' ' + requestId);
      });
  }

  // Every line (events, notes, connection status) lands in the single
  // timeline content container (`#events`), so a reconnect can clear
  // them all together.
  //
  // The timeline keeps the newest TIMELINE_MAX lines: each one is a
  // live DOM node, and a long-lived session (a chatty page's console
  // and network feed) would otherwise grow the document without bound
  // for as long as the dashboard stays open. Newest lines prepend, so
  // the ones trimmed off the end are the oldest.
  const TIMELINE_MAX = 500;

  function append(text) {
    const node = document.createElement('div');
    node.className = 'event';
    node.textContent = text;
    const list = document.getElementById('events');
    if (!list) { return; }
    list.prepend(node);
    while (list.children.length > TIMELINE_MAX) {
      list.removeChild(list.lastChild);
    }
  }
})();
