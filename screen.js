/* FretWright Bass Detect — screen, song-card action, job polling.
 *
 * Classic self-executing script, guarded so a re-hydration never registers the
 * card action twice or starts a second poller.
 */
(function () {
  'use strict';
  if (window.__fretwrightLoaded) return;
  window.__fretwrightLoaded = true;

  var API = '/api/plugins/fretwright';
  var SCREEN_ID = 'plugin-fretwright';
  var fb = window.feedBack || {};
  var state = { jobs: [], seen: {}, timer: null, search: '', searchTimer: null, status: null };

  function $(id) { return document.getElementById(id); }
  function esc(s) {
    return String(s == null ? '' : s).replace(/[&<>"']/g, function (c) {
      return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c];
    });
  }
  function api(path, body) {
    var opts = body === undefined ? {} : {
      method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body),
    };
    return fetch(API + path, opts).then(function (r) { return r.json(); });
  }
  function toast(title, message, accent) {
    if (window.fbNotify && window.fbNotify.show) {
      return window.fbNotify.show({ title: title, message: message || '', accent: accent || 'info' });
    }
    console.log('[fretwright]', title, message || '');
    return null;
  }
  function navigate(id) {
    if (fb.navigate) fb.navigate(id);
    else if (window.showScreen) window.showScreen(id);
  }

  // ── actions ────────────────────────────────────────────────────────────────
  function detect(filename) {
    return api('/detect', { filename: filename }).then(function (res) {
      if (res.error) { toast('Bass detection', res.error, 'error'); return; }
      toast('Detecting the bass line', 'It appears on the Bass Detect screen as it runs.', 'info');
      poll();
    });
  }

  function openInEditor(filename, index) {
    if (typeof window.editSong !== 'function') {
      toast('Song Editor not installed', 'Enable the Arrangement Editor plugin to edit the result.', 'warn');
      return;
    }
    // The editor reads this once the song has loaded and switches to our arrangement.
    window._editorPendingView = { filename: filename, arrangement: index, barSel: null };
    window.editSong(filename);
  }

  function play(filename, index) {
    if (typeof window.playSong === 'function') window.playSong(filename, index);
  }

  // ── song-card menu ─────────────────────────────────────────────────────────
  (function registerCardAction() {
    var reg = (window.feedBack && window.feedBack.libraryCardActions)
      || (window.slopsmith && window.slopsmith.libraryCardActions);
    if (!reg) return;
    reg.register({
      id: 'fretwright.detect-bass',
      pluginId: 'fretwright',
      label: 'Detect bass line (FretWright)',
      placement: 'menu',
      order: 16, // just under the editor's "Open in editor" (15)
      applies: function (song) { return !!(song && song.filename); },
      run: function (song) { return detect(song.filename); },
    });
  })();

  // ── jobs ───────────────────────────────────────────────────────────────────
  function renderJobs() {
    var el = $('fw-jobs');
    if (!el || !state.jobs.length) return;
    el.innerHTML = state.jobs.map(function (j) {
      var name = esc(j.filename.split('/').pop());
      var pct = Math.round((j.progress || 0) * 100);
      var pill = '<span class="fw-pill ' + esc(j.status) + '">' + esc(j.status) + '</span>';
      var actions = '';
      if (j.status === 'done') {
        actions = '<button class="fw-btn" data-edit="' + esc(j.filename) + '" data-idx="' + j.arrangement_index + '">Open in editor</button>'
          + '<button class="fw-btn secondary" data-play="' + esc(j.filename) + '" data-idx="' + j.arrangement_index + '">Play</button>';
      } else if (j.status === 'queued' || j.status === 'running') {
        actions = '<button class="fw-btn secondary" data-cancel="' + esc(j.id) + '">Cancel</button>';
      }
      var detail = esc(j.message || '');
      if (j.status === 'done' && j.seconds) detail += ' in ' + j.seconds + ' s';
      var warn = (j.warnings || []).map(function (w) { return '<div class="fw-muted">⚠ ' + esc(w) + '</div>'; }).join('');
      var bar = j.status === 'running'
        ? '<div class="fw-bar"><div style="width:' + pct + '%"></div></div>' : '';
      var err = j.status === 'failed' ? ' fw-error' : '';
      return '<div class="fw-job"><div class="fw-row">' + pill + '<span class="fw-name">' + name + '</span>'
        + '<span class="fw-detail' + err + '">' + detail + '</span>' + actions + '</div>' + bar + warn + '</div>';
    }).join('');
  }

  function poll() {
    if (state.timer) return;
    var tick = function () {
      api('/jobs').then(function (res) {
        state.jobs = res.jobs || [];
        renderJobs();
        var active = false;
        state.jobs.forEach(function (j) {
          if (j.status === 'queued' || j.status === 'running') active = true;
          var prev = state.seen[j.id];
          if (prev && prev !== j.status && j.status === 'done') {
            var card = toast('Bass line ready', j.filename.split('/').pop() + ': ' + j.message + '. Click to open in the editor.', 'success');
            if (card) {
              card.style.cursor = 'pointer';
              card.addEventListener('click', function () { openInEditor(j.filename, j.arrangement_index); });
            }
          } else if (prev && prev !== j.status && j.status === 'failed') {
            toast('Bass detection failed', j.message, 'error');
          }
          state.seen[j.id] = j.status;
        });
        state.timer = active ? setTimeout(tick, 1000) : null;
      }).catch(function () { state.timer = setTimeout(tick, 3000); });
    };
    state.timer = setTimeout(tick, 0);
  }

  // ── status + songs ─────────────────────────────────────────────────────────
  function loadStatus() {
    return api('/status').then(function (s) {
      state.status = s;
      var cli = s.cli;
      $('fw-cli').textContent = cli && cli.version ? cli.version : 'not found';
      $('fw-cli-help').hidden = !!cli;
      $('fw-source').textContent = s.config.own_separation
        ? 'feedBack bass stem, else FretWright Demucs' : 'feedBack bass stem';
    });
  }

  function loadSongs() {
    var el = $('fw-songs');
    api('/songs?size=100&q=' + encodeURIComponent(state.search)).then(function (res) {
      var songs = res.songs || [];
      if (!songs.length) { el.innerHTML = '<p class="fw-muted">No songs found.</p>'; return; }
      var own = state.status && state.status.config.own_separation;
      el.innerHTML = songs.map(function (s) {
        var stem = s.has_bass_stem
          ? '<span class="fw-pill done">bass stem</span>'
          : '<span class="fw-pill queued">no stem</span>';
        var disabled = s.has_bass_stem || own ? '' : ' disabled title="Split stems first (Stem Splitter)"';
        return '<div class="fw-job"><div class="fw-row">' + stem
          + '<span class="fw-name"><strong>' + esc(s.title || s.filename) + '</strong> <span class="fw-muted">' + esc(s.artist) + '</span></span>'
          + '<button class="fw-btn" data-detect="' + esc(s.filename) + '"' + disabled + '>Detect bass line</button></div></div>';
      }).join('');
    });
  }

  function onClick(e) {
    var t = e.target.closest('button');
    if (!t) return;
    if (t.dataset.detect) detect(t.dataset.detect);
    else if (t.dataset.edit) openInEditor(t.dataset.edit, Number(t.dataset.idx));
    else if (t.dataset.play) play(t.dataset.play, Number(t.dataset.idx));
    else if (t.dataset.cancel) api('/jobs/' + t.dataset.cancel + '/cancel', {}).then(poll);
    else if (t.id === 'fw-open-settings') navigate('settings');
  }

  function init() {
    var root = document.querySelector('#' + SCREEN_ID + ' .fretwright');
    if (!root || root.dataset.bound) return;
    root.dataset.bound = '1';
    root.addEventListener('click', onClick);
    $('fw-search').addEventListener('input', function (e) {
      clearTimeout(state.searchTimer);
      state.searchTimer = setTimeout(function () { state.search = e.target.value; loadSongs(); }, 250);
    });
  }

  function onShow() {
    init();
    loadStatus().then(loadSongs);
    poll();
  }

  if (fb.on) {
    fb.on('screen:changed', function (ev) {
      var id = ev && (ev.id || ev.screen || (ev.detail && ev.detail.id));
      if (id === SCREEN_ID) onShow();
    });
  }
  if ($(SCREEN_ID) && $(SCREEN_ID).classList.contains('active')) onShow();
})();
