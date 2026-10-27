/* ============================================================================
   Marguerite 官网 · 85 秒卡点短片引擎

   时间轴由音频驱动：audio.currentTime 就是时钟，因此画面、打字、光标与
   底部进度天然同步。拍点来自 tools/analyze_audio.py 对 focus-pulse.mp3 的
   实测（56 BPM，相位 0.778s）；beat-map.json 缺失时用同一组常量兜底。

   边界：
   - 浏览器拦截自动播放 → 退回 performance.now() 时钟，并在封面上如实提示；
   - prefers-reduced-motion → 不自动播放，展示静帧与说明；
   - 不向任何服务器发送内容（纯本地）。
   ========================================================================== */
(function () {
  'use strict';

  var stage = document.getElementById('playerStage');
  var player = document.getElementById('player');
  if (!stage || !player) return;

  var audio = document.getElementById('soundtrack');
  var cover = player.querySelector('[data-cover]');
  var hint = player.querySelector('[data-audio-hint]');
  var fallback = player.querySelector('[data-fallback]');
  var progressEl = player.querySelector('[data-progress]');
  var clockEl = player.querySelector('[data-clock]');
  var nameEl = player.querySelector('[data-act-name]');
  var soundBtn = document.getElementById('soundToggle');
  var soundLabel = soundBtn ? soundBtn.querySelector('[data-sound-label]') : null;

  var reduceMQ = window.matchMedia('(prefers-reduced-motion: reduce)');
  var acts = Array.prototype.slice.call(stage.querySelectorAll('.act')).map(function (el) {
    return {
      el: el,
      key: el.getAttribute('data-act'),
      start: parseFloat(el.getAttribute('data-start')),
      end: parseFloat(el.getAttribute('data-end')),
      label: {
        hook: '01 · 开场',
        chat: '02 · 真实会话',
        settings: '03 · 调成你的',
        diff: '04 · 为何不同',
        install: '05 · 像看电影一样安装',
        brand: '06 · 品牌'
      }[el.getAttribute('data-act')] || ''
    };
  });

  var TIMING = { duration: 85, beatSeconds: 1.0714, beatPhase: 0.778 };
  // beat-map.js 由 tools/analyze_audio.py 生成（<script> 读取，file:// 下也成立）。
  if (window.MARGUERITE_BEATS) {
    var beats = window.MARGUERITE_BEATS;
    TIMING.duration = beats.duration || TIMING.duration;
    TIMING.beatSeconds = beats.beatSeconds || TIMING.beatSeconds;
    TIMING.beatPhase = beats.beatPhase || TIMING.beatPhase;
    stage.setAttribute('data-bpm', String(Math.round(beats.beatBpm || 56)));
  }

  /* ---------- 品牌场景：克隆一次给最后一幕用 ---------- */
  var brandSource = document.getElementById('brandScene');
  var brandSlot = stage.querySelector('[data-brand-slot]');
  if (brandSource && brandSlot && !brandSlot.firstElementChild) {
    var clone = brandSource.cloneNode(true);
    clone.removeAttribute('id');
    clone.classList.remove('scene--section');
    clone.classList.add('scene--act');
    var svg = clone.querySelector('svg');
    if (svg) {
      Array.prototype.forEach.call(svg.querySelectorAll('[id]'), function (node) {
        node.removeAttribute('id');
      });
    }
    brandSlot.appendChild(clone);
  }
  var brandCloneFlower = brandSlot ? brandSlot.querySelector('.flower') : null;

  /* ---------- 状态 ---------- */
  var state = {
    playing: false,
    ended: false,
    blocked: false,
    startedAt: 0,
    raf: 0,
    lastAct: null,
    lastBeat: -1,
    lastTyped: -1,
    soundOn: false
  };

  // 封面态也要有画面：先亮出第 1 幕，否则播放器是一块纯黑。
  if (acts.length) {
    acts[0].el.classList.add('is-active');
    state.lastAct = acts[0];
    if (nameEl) nameEl.textContent = acts[0].label;
  }

  function now() {
    if (!state.blocked && audio && audio.readyState >= 2 && !audio.paused) {
      return audio.currentTime;
    }
    return (performance.now() - state.startedAt) / 1000;
  }

  function clamp01(v) { return v < 0 ? 0 : v > 1 ? 1 : v; }
  function pad(n) { return n < 10 ? '0' + n : String(n); }

  function setActive(act) {
    if (state.lastAct === act) return;
    state.lastAct = act;
    acts.forEach(function (item) { item.el.classList.toggle('is-active', item === act); });
    if (act) {
      nameEl.textContent = act.label;
      act.el.classList.add('is-live');
      acts.forEach(function (item) { if (item !== act) item.el.classList.remove('is-live'); });
      if (act.key === 'brand' && brandCloneFlower) {
        brandCloneFlower.classList.add('is-bloom');
      }
      if (act.key === 'diff') {
        var cells = act.el.querySelectorAll('.diff__cell');
        Array.prototype.forEach.call(cells, function (cell, i) {
          cell.style.opacity = '0';
          cell.style.transform = 'translateY(14px)';
          cell.style.transition = 'opacity .5s cubic-bezier(.22,1,.36,1) ' + (i * 0.18) + 's, transform .5s cubic-bezier(.22,1,.36,1) ' + (i * 0.18) + 's';
        });
      }
    }
  }

  var TYPE_TEXT = '把这次排查的结论整理成一页纸：根因、证据链、影响面、下一步。';

  function updateAct(act, t) {
    var span = Math.max(0.001, act.end - act.start);
    var p = clamp01((t - act.start) / span);

    if (act.key === 'chat') {
      var frame = act.el.querySelector('.shotframe__img');
      if (frame) frame.style.setProperty('--z', String(1.02 + p * 0.1));
      var chars = Math.round(clamp01((p - 0.04) / 0.32) * TYPE_TEXT.length);
      if (chars !== state.lastTyped) {
        state.lastTyped = chars;
        var typer = act.el.querySelector('[data-typer]');
        if (typer) typer.textContent = TYPE_TEXT.slice(0, chars);
      }
    } else if (act.key === 'settings') {
      var img = act.el.querySelector('.shotframe__img');
      if (img) img.style.setProperty('--z', String(1.12 - p * 0.1));
      var cursor = act.el.querySelector('[data-cursor]');
      var ripple = act.el.querySelector('[data-ripple]');
      if (cursor) {
        var x = 46 + p * 14;
        var y = 44 + Math.sin(p * Math.PI) * 6;
        cursor.style.left = x + '%';
        cursor.style.top = y + '%';
        cursor.classList.toggle('is-on', p > 0.08);
      }
      if (ripple) {
        var firing = p > 0.42 && p < 0.62;
        ripple.style.left = '60%';
        ripple.style.top = '44%';
        if (firing && !ripple.classList.contains('is-on')) ripple.classList.add('is-on');
        if (!firing) ripple.classList.remove('is-on');
      }
    } else if (act.key === 'diff') {
      var cells = act.el.querySelectorAll('.diff__cell');
      Array.prototype.forEach.call(cells, function (cell, i) {
        var at = 0.06 + i * 0.12;
        var on = p >= at;
        cell.style.opacity = on ? '1' : '0';
        cell.style.transform = on ? 'none' : 'translateY(14px)';
      });
    } else if (act.key === 'brand') {
      if (brandCloneFlower) brandCloneFlower.classList.add('is-bloom');
    }
  }

  function pulseBeat(t) {
    var index = Math.floor((t - TIMING.beatPhase) / TIMING.beatSeconds);
    if (index !== state.lastBeat) {
      state.lastBeat = index;
      stage.classList.add('is-beat');
      window.clearTimeout(pulseBeat.timer);
      pulseBeat.timer = window.setTimeout(function () { stage.classList.remove('is-beat'); }, 130);
    }
  }

  function tick() {
    if (!state.playing) return;
    var t = Math.min(now(), TIMING.duration);
    var active = null;
    for (var i = 0; i < acts.length; i++) {
      if (t >= acts[i].start && t < acts[i].end) { active = acts[i]; break; }
    }
    if (!active) active = acts[acts.length - 1];
    setActive(active);
    updateAct(active, t);
    pulseBeat(t);

    progressEl.style.width = ((t / TIMING.duration) * 100).toFixed(2) + '%';
    clockEl.textContent = '0' + Math.floor(t / 60) + ':' + pad(Math.floor(t % 60));

    if (t >= TIMING.duration - 0.05) {
      finish();
      return;
    }
    state.raf = window.requestAnimationFrame(tick);
  }

  function start() {
    if (reduceMQ.matches) return;
    state.playing = true;
    state.ended = false;
    state.blocked = false;
    state.lastAct = null;
    state.lastBeat = -1;
    state.lastTyped = -1;
    player.setAttribute('data-state', 'playing');
    cover.hidden = true;
    fallback.hidden = true;

    stage.querySelectorAll('.act').forEach(function (el) { el.classList.remove('is-live'); });
    if (clockEl) clockEl.textContent = '00:00';
    if (nameEl) nameEl.textContent = '01 · 开场';

    if (audio) {
      audio.loop = false;
      audio.currentTime = 0;
      audio.muted = false;
      var promise = audio.play();
      if (promise && typeof promise.catch === 'function') {
        promise.catch(function () {
          state.blocked = true;
          if (hint) hint.hidden = false;
          setSound(true, true);
        });
      }
    } else {
      state.blocked = true;
    }

    state.startedAt = performance.now();
    window.cancelAnimationFrame(state.raf);
    state.raf = window.requestAnimationFrame(tick);
  }

  function finish() {
    state.playing = false;
    state.ended = true;
    window.cancelAnimationFrame(state.raf);
    if (audio) {
      audio.pause();
      audio.currentTime = 0;
    }
    player.setAttribute('data-state', 'ended');
    cover.hidden = false;
    var title = cover.querySelector('.cover__title');
    if (title) title.textContent = '再看一遍 · 85 秒';
    if (brandCloneFlower) brandCloneFlower.classList.add('is-bloom');
    if (progressEl) progressEl.style.width = '100%';
    if (clockEl) clockEl.textContent = '01:25';
    if (nameEl) nameEl.textContent = '06 · 品牌';
  }

  function stop() {
    state.playing = false;
    window.cancelAnimationFrame(state.raf);
    if (audio) audio.pause();
    player.setAttribute('data-state', 'idle');
    cover.hidden = false;
    stage.querySelectorAll('.act').forEach(function (el) { el.classList.remove('is-active', 'is-live'); });
  }

  if (cover) {
    cover.addEventListener('click', function () { start(); });
    cover.addEventListener('keydown', function (event) {
      if (event.key === 'Enter' || event.key === ' ') {
        event.preventDefault();
        start();
      }
    });
    cover.tabIndex = 0;
    cover.setAttribute('role', 'button');
    cover.setAttribute('aria-label', '播放 85 秒卡点短片');
  }

  stage.addEventListener('keydown', function (event) {
    if (event.key === ' ' || event.key === 'Enter') {
      event.preventDefault();
      if (state.playing) stop(); else start();
    } else if (event.key === 'Escape') {
      stop();
    }
  });

  /* ---------- 声音（独立于短片，可当环境音循环） ---------- */
  function setSound(on, silent) {
    state.soundOn = on;
    if (soundBtn) {
      soundBtn.setAttribute('aria-pressed', String(on));
      soundBtn.setAttribute('aria-label', on ? '关闭背景音乐 focus-pulse' : '开启背景音乐 focus-pulse');
    }
    if (soundLabel) soundLabel.textContent = on ? '声音开' : '声音关';
    if (!audio) return;
    audio.muted = !on;
    if (on) {
      if (!state.playing) { audio.loop = true; if (audio.currentTime > 84.5) audio.currentTime = 0; }
      var promise = audio.play();
      if (promise && promise.catch) {
        promise.catch(function () {
          if (!silent && soundBtn) {
            soundBtn.setAttribute('aria-pressed', 'false');
            if (soundLabel) soundLabel.textContent = '声音关';
            state.soundOn = false;
          }
        });
      }
    } else if (!state.playing) {
      audio.pause();
    }
  }

  if (soundBtn) {
    soundBtn.addEventListener('click', function () { setSound(!state.soundOn); });
  }

  /* ---------- 入口：hero 的「观看 85 秒」 ---------- */
  var starter = document.querySelector('[data-start-film]');
  if (starter) {
    starter.addEventListener('click', function (event) {
      event.preventDefault();
      document.getElementById('film').scrollIntoView({ behavior: 'smooth', block: 'center' });
      window.setTimeout(start, 420);
    });
  }

  /* ---------- 减少动态效果：静帧 + 说明 ---------- */
  function applyReducedMotion() {
    if (!reduceMQ.matches) return;
    player.setAttribute('data-state', 'reduced');
    if (cover) cover.hidden = true;
    if (fallback) fallback.hidden = false;
    var first = acts[0];
    if (first) first.el.classList.add('is-active');
    setSound(false, true);
  }
  applyReducedMotion();
  if (reduceMQ.addEventListener) {
    reduceMQ.addEventListener('change', function () {
      if (reduceMQ.matches) {
        stop();
        applyReducedMotion();
      } else {
        player.setAttribute('data-state', 'idle');
        if (cover) cover.hidden = false;
        if (fallback) fallback.hidden = true;
      }
    });
  }

  /* ---------- 品牌场景：进入视口时绽放 ---------- */
  var brandFlower = brandSource ? brandSource.querySelector('.flower') : null;
  if (brandFlower && 'IntersectionObserver' in window) {
    var io = new IntersectionObserver(function (entries) {
      entries.forEach(function (entry) {
        if (entry.isIntersecting) {
          brandFlower.classList.add('is-bloom');
          io.unobserve(brandFlower);
        }
      });
    }, { threshold: 0.35 });
    io.observe(brandFlower);
  }
})();
