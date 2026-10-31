/* ============================================================================
   Marguerite 官网 · 渐进增强脚本
   本站在禁用 JS 时功能完整。本文件只补五件事：移动端菜单、命令复制、
   公告条的关闭记忆、窄屏常驻下载条、首屏提问带进下面的真实演示。
   复制做三段降级：Clipboard API → execCommand → 明确报失败（绝不静默吞错）。
   ========================================================================== */
(function () {
  'use strict';

  var ICON_COPY =
    '<svg class="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" ' +
    'stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false">' +
    '<rect x="9" y="9" width="12" height="12" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>';
  var ICON_DONE =
    '<svg class="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" ' +
    'stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false"><path d="M20 6 9 17l-5-5"/></svg>';
  var RESET = 1800;

  /* ---- 移动端菜单 ---- */
  var toggle = document.getElementById('navToggle');
  var menu = document.getElementById('navMenu');
  if (toggle && menu) {
    function setOpen(open) {
      menu.hidden = !open;
      toggle.setAttribute('aria-expanded', String(open));
      toggle.setAttribute('aria-label', open ? '关闭菜单' : '打开菜单');
    }
    toggle.addEventListener('click', function () { setOpen(menu.hidden); });
    menu.addEventListener('click', function (e) {
      if (e.target.tagName === 'A') setOpen(false);
    });
    document.addEventListener('keydown', function (e) { if (e.key === 'Escape') setOpen(false); });
  }

  /* ---- 命令复制 ---- */
  function copyExec(text) {
    var ta = document.createElement('textarea');
    ta.value = text;
    ta.setAttribute('readonly', '');
    ta.style.position = 'fixed';
    ta.style.top = '-1000px';
    ta.style.opacity = '0';
    document.body.appendChild(ta);
    var ok = false;
    try { ta.select(); ta.setSelectionRange(0, ta.value.length); ok = document.execCommand('copy'); }
    catch (err) { ok = false; }
    finally { document.body.removeChild(ta); }
    return ok;
  }
  function copyText(text) {
    var async = typeof navigator !== 'undefined' && navigator.clipboard &&
      typeof navigator.clipboard.writeText === 'function' && window.isSecureContext;
    if (!async) return Promise.resolve(copyExec(text));
    return navigator.clipboard.writeText(text).then(function () { return true; },
      function () { return copyExec(text); });
  }
  function setState(btn, label, icon, state) {
    btn.innerHTML = icon + '<span>' + label + '</span>';
    btn.setAttribute('data-state', state);
  }
  function attach(pre, i) {
    var code = pre.querySelector('code');
    if (!code) return;
    var src = code.textContent;
    if (!src || !src.trim()) return;
    var btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'copy';
    btn.setAttribute('aria-label', '复制这段命令');
    setState(btn, '复制', ICON_COPY, 'idle');
    var live = document.createElement('span');
    live.className = 'sr-only';
    live.setAttribute('role', 'status');
    live.setAttribute('aria-live', 'polite');
    live.id = 'copy-status-' + i;
    var timer = null;
    btn.addEventListener('click', function () {
      if (timer) { window.clearTimeout(timer); timer = null; }
      copyText(src).then(function (ok) {
        if (ok) { setState(btn, '已复制', ICON_DONE, 'done'); live.textContent = '命令已复制到剪贴板'; }
        else { setState(btn, '复制失败', ICON_COPY, 'fail'); live.textContent = '复制失败，请手动选择命令文本'; }
        timer = window.setTimeout(function () {
          setState(btn, '复制', ICON_COPY, 'idle'); live.textContent = ''; timer = null;
        }, RESET);
      });
    });
    var shell = document.createElement('div');
    shell.className = 'code-shell';
    if (pre.parentNode) {
      pre.parentNode.insertBefore(shell, pre);
      shell.appendChild(pre);
      shell.appendChild(btn);
      shell.appendChild(live);
    }
  }

  var blocks = document.querySelectorAll('pre.code');
  for (var i = 0; i < blocks.length; i++) attach(blocks[i], i);

  /* ---- 顶部公告条：关闭后记在本地，下次不再占位 ---- */
  (function notice() {
    var bar = document.querySelector('[data-notice]');
    if (!bar) return;
    var KEY = 'marguerite.notice.v1';
    try { if (window.localStorage.getItem(KEY) === 'closed') bar.hidden = true; } catch (err) {}
    var close = bar.querySelector('[data-notice-close]');
    if (close) {
      close.addEventListener('click', function () {
        bar.hidden = true;
        try { window.localStorage.setItem(KEY, 'closed'); } catch (err) {}
      });
    }
  })();

  /* ---- 窄屏常驻下载条：过首屏出现；遇到播放器/演示/下载/页脚就收起，别压住这些区的按钮 ---- */
  (function dock() {
    var el = document.querySelector('[data-dock]');
    if (!el) return;
    var mq = window.matchMedia('(max-width:1023px)');
    var zones = ['film', 'demo', 'download'].map(function (id) { return document.getElementById(id); });
    var foot = document.querySelector('.foot');
    if (foot) zones.push(foot);
    zones = zones.filter(Boolean);
    var pending = false;
    function update() {
      pending = false;
      if (!mq.matches) { el.classList.remove('is-on'); return; }
      var hero = document.querySelector('.hero');
      var passedHero = window.pageYOffset > (hero ? hero.offsetHeight * 0.9 : 600);
      var vh = window.innerHeight;
      var blocked = zones.some(function (z) {
        var r = z.getBoundingClientRect();
        return r.top < vh * 0.82 && r.bottom > vh * 0.55;
      });
      el.classList.toggle('is-on', passedHero && !blocked);
    }
    function onScroll() {
      if (pending) return;
      pending = true;
      window.requestAnimationFrame(update);
    }
    window.addEventListener('scroll', onScroll, { passive: true });
    window.addEventListener('resize', onScroll, { passive: true });
    update();
  })();

  /* ---- 首屏提问：把这句话带进下面的真实演示输入框并跑一次 ---- */
  (function ask() {
    var form = document.querySelector('[data-ask]');
    var field = document.querySelector('[data-ask-input]');
    var demo = document.getElementById('demo');
    var input = document.querySelector('[data-demo-input]');
    var send = document.querySelector('[data-demo-send]');
    if (!form || !field || !demo || !input || !send) return;

    form.addEventListener('submit', function (event) {
      event.preventDefault();
      var text = (field.value || '').trim() || input.getAttribute('placeholder') || '';
      var reduce = window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
      input.value = text;
      demo.scrollIntoView({ behavior: reduce ? 'auto' : 'smooth', block: 'start' });
      window.setTimeout(function () {
        send.click();
        try { input.focus({ preventScroll: true }); } catch (err) { input.focus(); }
      }, reduce ? 90 : 700);
    });
  })();

  /* ---- 操作演示：真实输入 + 工具回执 + 流式生成 ---- */
  (function demo() {
    var input = document.querySelector('[data-demo-input]');
    var send = document.querySelector('[data-demo-send]');
    var thread = document.querySelector('[data-demo-thread]');
    var empty = document.querySelector('[data-demo-empty]');
    var exampleBtn = document.querySelector('[data-demo-example]');
    if (!input || !send || !thread) return;

    var EXAMPLE = '把这次排查的结论整理成一页纸：根因、证据链、影响面、下一步。';
    var EXAMPLE_SHORT = '只回答一句：这次排查最该先修什么？';
    var ANSWER = [
      '结论：这次排查没有形成可交付的结论。',
      '子专家没有返回对项目本身的实质分析：其中一个执行失败，另外两个只停留在查看结构阶段。',
      '',
      '下一步（按顺序）：',
      '1. 先把会话绑定到真实项目目录，让工具读到实际文件；',
      '2. 按「依赖 → 配置 → 代码质量」三段重跑，每段都要留下可复核的证据行；',
      '3. 仍然拿不到结论时，把失败的那一条单独复现，不要整轮重来。'
    ].join('\n');
    var ANSWER_SHORT = '先修「会话没有绑定真实项目目录」——这是这次排查拿不到结论的唯一根因。';
    var busy = false;
    var timer = null;

    function append(className, text) {
      var el = document.createElement('div');
      el.className = className;
      if (text) el.textContent = text;
      thread.appendChild(el);
      thread.scrollTop = thread.scrollHeight;
      return el;
    }

    function stream(el, text, done) {
      var i = 0;
      var caret = document.createElement('span');
      caret.className = 'caret';
      caret.setAttribute('aria-hidden', 'true');
      el.appendChild(caret);
      timer = window.setInterval(function () {
        i += 1;
        el.textContent = text.slice(0, i);
        el.appendChild(caret);
        thread.scrollTop = thread.scrollHeight;
        if (i >= text.length) {
          window.clearInterval(timer);
          timer = null;
          caret.remove();
          busy = false;
          send.disabled = false;
          send.textContent = '发送';
          if (done) done();
        }
      }, 78);
    }

    function run() {
      if (busy) return;
      var prompt = (input.value || '').trim() || EXAMPLE;
      input.value = prompt;
      if (empty && empty.parentNode) empty.remove();

      busy = true;
      send.disabled = true;
      send.textContent = '生成中';

      append('demo__msg-u', prompt);
      input.value = '';

      window.setTimeout(function () {
        var tool = append('demo__tool');
        tool.textContent = 'web_search · 检索 4 条来源 · 2.1s · ok';
        window.requestAnimationFrame(function () { tool.classList.add('in'); });

        window.setTimeout(function () {
          var answer = append('demo__msg-a');
          var text = prompt.length <= 18 ? ANSWER_SHORT : ANSWER;
          stream(answer, text);
        }, 720);
      }, 520);
    }

    send.addEventListener('click', run);
    input.addEventListener('keydown', function (event) {
      if ((event.ctrlKey || event.metaKey) && event.key === 'Enter') {
        event.preventDefault();
        run();
      }
    });
    if (exampleBtn) {
      exampleBtn.addEventListener('click', function () {
        input.value = input.value.trim() === EXAMPLE ? EXAMPLE_SHORT : EXAMPLE;
        input.focus();
      });
    }
  })();

  /* ---- 首屏命令行打字（Qoder/CNB 式终端气质；降级时直接显示全文） ---- */
  (function cmdline() {
    var el = document.querySelector('[data-cmdline]');
    if (!el) return;
    var full = el.getAttribute('data-cmdline') || '';
    var reduce = window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    if (reduce || !full) return;
    el.textContent = '';
    var i = 0;
    var timer = window.setInterval(function () {
      i += 1;
      el.textContent = full.slice(0, i);
      if (i >= full.length) { window.clearInterval(timer); }
    }, 52);
  })();

  /* ---- 四入口 Tab：编号切换（Trae 式），键盘左右可移焦 ---- */
  (function entries() {
    var tabs = Array.prototype.slice.call(document.querySelectorAll('.entries__tab'));
    if (!tabs.length) return;
    function activate(tab) {
      tabs.forEach(function (t) {
        var on = t === tab;
        t.classList.toggle('is-active', on);
        t.setAttribute('aria-selected', String(on));
        var panel = document.getElementById(t.getAttribute('aria-controls') || '');
        if (panel) panel.classList.toggle('is-active', on);
      });
    }
    tabs.forEach(function (tab, idx) {
      tab.addEventListener('click', function () { activate(tab); });
      tab.addEventListener('keydown', function (event) {
        var delta = event.key === 'ArrowRight' ? 1 : (event.key === 'ArrowLeft' ? -1 : 0);
        if (!delta) return;
        event.preventDefault();
        var next = tabs[(idx + delta + tabs.length) % tabs.length];
        next.focus();
        activate(next);
      });
    });
  })();

  /* ---- 证据数字滚动：进入视口才从 0 数到目标值 ---- */
  (function counters() {
    var nums = Array.prototype.slice.call(document.querySelectorAll('[data-count]'));
    if (!nums.length) return;
    var reduce = window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    if (reduce || !('IntersectionObserver' in window)) return; // 降级：HTML 里已是终值
    var io = new IntersectionObserver(function (entries) {
      entries.forEach(function (entry) {
        if (!entry.isIntersecting) return;
        io.unobserve(entry.target);
        var target = parseInt(entry.target.getAttribute('data-count'), 10) || 0;
        var t0 = null;
        var DUR = 900;
        function step(ts) {
          if (t0 === null) t0 = ts;
          var p = Math.min(1, (ts - t0) / DUR);
          var eased = 1 - Math.pow(1 - p, 3);
          entry.target.textContent = String(Math.round(target * eased));
          if (p < 1) window.requestAnimationFrame(step);
        }
        entry.target.textContent = '0';
        window.requestAnimationFrame(step);
      });
    }, { threshold: 0.4 });
    nums.forEach(function (n) { io.observe(n); });
  })();

  /* ---- 能力卡鼠标聚光：只写两个 CSS 变量，桌面细指针才启用 ---- */
  (function spotlight() {
    var fine = window.matchMedia && window.matchMedia('(pointer: fine)').matches;
    var reduce = window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    if (!fine || reduce) return;
    var cards = Array.prototype.slice.call(document.querySelectorAll('.card'));
    cards.forEach(function (card) {
      card.addEventListener('pointermove', function (event) {
        var r = card.getBoundingClientRect();
        card.style.setProperty('--mx', (event.clientX - r.left).toFixed(1) + 'px');
        card.style.setProperty('--my', (event.clientY - r.top).toFixed(1) + 'px');
      });
    });
  })();
})();

/* ── 导航当前位置（2026-10-29 补）──────────────────────────────────────────
   问题：styles.css 早就写好了 `.nav__links a.is-current`（正文色 + 2px 强调色下划线），
   但**没有任何 JS 去加这个 class**，`aria-current` 也是 0 处 —— 于是这套样式是死代码，
   用户在长页面里滚到哪一节，导航完全不给反馈；读屏用户更没有任何"当前在哪"的信号。

   做法：滚动时取「最后一个顶部已越过参考线」的 section，给对应链接加 `is-current`
   与 `aria-current="location"`（`location` 是 nav 里表示"当前视口位置"的合法值，
   不是表示"当前页面"的 `page`）。用 rAF 节流，避免 scroll 事件里做布局查询。
   无 JS / 脚本失败时导航只是没有高亮，功能不受影响。 */
(function () {
  var links = Array.prototype.slice.call(document.querySelectorAll('.nav__links a[href^="#"]'));
  if (!links.length) return;
  var pairs = links.map(function (a) {
    var id = a.getAttribute('href').slice(1);
    return { a: a, sec: id ? document.getElementById(id) : null };
  }).filter(function (p) { return !!p.sec; });
  if (!pairs.length) return;

  var current = null;
  var queued = false;

  function mark(active) {
    if (active === current) return;
    pairs.forEach(function (p) {
      var on = p.a === active;
      p.a.classList.toggle('is-current', on);
      if (on) p.a.setAttribute('aria-current', 'location');
      else p.a.removeAttribute('aria-current');
    });
    current = active;
  }

  function update() {
    queued = false;
    // 参考线取视口上方 1/3：读者视线通常落在上三分之一，用它能得到"正在读的那一节"。
    var line = window.innerHeight / 3;
    // 取「离参考线最近的、已越过的」那一节 —— 不能按导航顺序取"最后一个"：
    // 导航顺序与文档顺序**不一致**（实测 `#compare` 的 section 在 DOM 里排在 `#capabilities`
    // 之前），按顺序取会高亮错一整节。这里按几何距离取，与文档顺序无关。
    var best = null;
    var bestTop = -Infinity;
    for (var i = 0; i < pairs.length; i++) {
      var top = pairs[i].sec.getBoundingClientRect().top;
      if (top <= line && top > bestTop) { bestTop = top; best = pairs[i].a; }
    }
    // 滚到最底部时，最后一节可能永远越不过参考线 —— 显式兜底，避免"到底了却没高亮"。
    if (window.innerHeight + window.scrollY >= document.body.scrollHeight - 4) {
      best = pairs[pairs.length - 1].a;
    }
    mark(best);
  }

  function onScroll() {
    if (queued) return;
    queued = true;
    window.requestAnimationFrame(update);
  }

  window.addEventListener('scroll', onScroll, { passive: true });
  window.addEventListener('resize', onScroll, { passive: true });
  update();
})();


/* ==== 2026-10-31 · 战术推演 + 证据模式 ==== */

/* ── 战术推演台（2026-10-31）────────────────────────────────────────────────
   这里只演示机制，不产生任何可引用数字：
   · 五种拓扑的节点/连线是示意，不是引擎输出的 DAG；
   · 失败节点由固定种子选出，闸门是否拦下只按 model.rs 里的检出率掷一次骰子；
   · 真实胜率必须在应用内跑 `tactics.simulate`（七条战术 × 2,000 次试验）。
   把这条边界写死在 UI 与注释里，是为了避免「官网示意图被当成 benchmark」。 */
(function () {
  'use strict';
  var svg = document.querySelector('[data-tac-svg]');
  var statusEl = document.querySelector('[data-tac-status]');
  var logEl = document.querySelector('[data-tac-log]');
  var runBtn = document.querySelector('[data-tac-run]');
  var gateEl = document.querySelector('[data-tac-gate]');
  var gateDesc = document.querySelector('[data-tac-gate-desc]');
  var hintEl = document.querySelector('[data-tac-hint]');
  var tabs = Array.prototype.slice.call(document.querySelectorAll('.tac__tab'));
  var chips = Array.prototype.slice.call(document.querySelectorAll('.tac__chip'));
  if (!svg || !statusEl || !runBtn || !gateEl) return;

  var NS = 'http://www.w3.org/2000/svg';
  var TOPO = {
    solo: {
      name: '单人拓扑',
      aria: '单人拓扑：一个执行者依次跑完四个节点',
      gate: '检出率 85% · 误报 5% · token +10% · 耗时 ×1.15',
      detect: 0.85,
      nodes: [{ x: 320, y: 46, l: '01' }, { x: 320, y: 108, l: '02' }, { x: 320, y: 170, l: '03' }, { x: 320, y: 232, l: '04' }],
      edges: [[0, 1], [1, 2], [2, 3]]
    },
    star: {
      name: '星型拓扑',
      aria: '星型拓扑：主智能体把三个子任务分给三个子智能体，再汇总到一个整合节点',
      gate: '检出率 85% · 误报 5% · token +10% · 耗时 ×1.15',
      detect: 0.85,
      nodes: [{ x: 320, y: 42, l: '主' }, { x: 140, y: 140, l: '子1' }, { x: 320, y: 140, l: '子2' }, { x: 500, y: 140, l: '子3' }, { x: 320, y: 248, l: '整' }],
      edges: [[0, 1], [0, 2], [0, 3], [1, 4], [2, 4], [3, 4]]
    },
    panel: {
      name: '专家团拓扑',
      aria: '专家团拓扑：三位专家各自独立作答，再由选择器合成结果',
      gate: '检出率 92% · 误报 6% · token +12% · 耗时 ×1.20',
      detect: 0.92,
      nodes: [{ x: 140, y: 66, l: 'A1' }, { x: 140, y: 148, l: 'A2' }, { x: 320, y: 66, l: 'B1' }, { x: 320, y: 148, l: 'B2' }, { x: 500, y: 66, l: 'C1' }, { x: 500, y: 148, l: 'C2' }, { x: 320, y: 246, l: '选' }],
      edges: [[0, 6], [1, 6], [2, 6], [3, 6], [4, 6], [5, 6]]
    },
    swarm: {
      name: '蜂群拓扑',
      aria: '蜂群拓扑：一个分片器把任务切成六片并行执行，再由聚合器收口',
      gate: '检出率 80% · 误报 8% · token +8% · 耗时 ×1.10',
      detect: 0.80,
      nodes: [{ x: 320, y: 38, l: '分' }, { x: 90, y: 128, l: 'S1' }, { x: 182, y: 128, l: 'S2' }, { x: 274, y: 128, l: 'S3' }, { x: 366, y: 128, l: 'S4' }, { x: 458, y: 128, l: 'S5' }, { x: 550, y: 128, l: 'S6' }, { x: 320, y: 238, l: '聚' }],
      edges: [[0, 1], [0, 2], [0, 3], [0, 4], [0, 5], [0, 6], [1, 7], [2, 7], [3, 7], [4, 7], [5, 7], [6, 7]]
    },
    relay: {
      name: '接力拓扑',
      aria: '接力拓扑：五个专职执行者按顺序接力，每个节点只做一件事',
      gate: '检出率 88% · 误报 4% · token +9% · 耗时 ×1.12',
      detect: 0.88,
      nodes: [{ x: 80, y: 150, l: '01' }, { x: 200, y: 96, l: '02' }, { x: 320, y: 174, l: '03' }, { x: 440, y: 108, l: '04' }, { x: 560, y: 160, l: '05' }],
      edges: [[0, 1], [1, 2], [2, 3], [3, 4]]
    }
  };
  var state = { topo: 'solo', gate: true, seed: 41, running: false };
  var reduce = window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches;

  function svgEl(name, attrs) {
    var n = document.createElementNS(NS, name);
    for (var k in attrs) {
      if (Object.prototype.hasOwnProperty.call(attrs, k)) n.setAttribute(k, attrs[k]);
    }
    return n;
  }

  function setTone(tone) {
    if (tone) statusEl.setAttribute('data-tone', tone);
    else statusEl.removeAttribute('data-tone');
  }

  function line(text, tone) {
    if (!logEl) return;
    var p = document.createElement('p');
    p.className = 'tac__line';
    if (tone) p.setAttribute('data-tone', tone);
    p.textContent = text;
    logEl.appendChild(p);
    while (logEl.children.length > 4) logEl.removeChild(logEl.firstChild);
  }

  function clearMarks() {
    var ns = svg.querySelectorAll('.tac__node');
    for (var i = 0; i < ns.length; i++) ns[i].classList.remove('is-run', 'is-fail', 'is-caught');
    var es = svg.querySelectorAll('.tac__edge');
    for (var j = 0; j < es.length; j++) es[j].classList.remove('is-flow');
  }

  function render() {
    var spec = TOPO[state.topo];
    while (svg.firstChild) svg.removeChild(svg.firstChild);
    spec.edges.forEach(function (e) {
      var a = spec.nodes[e[0]];
      var b = spec.nodes[e[1]];
      svg.appendChild(svgEl('line', { x1: a.x, y1: a.y, x2: b.x, y2: b.y, 'class': 'tac__edge' }));
    });
    spec.nodes.forEach(function (nd, i) {
      var g = svgEl('g', { 'class': 'tac__node', 'data-node': String(i) });
      g.appendChild(svgEl('circle', { cx: nd.x, cy: nd.y, r: 18 }));
      var t = svgEl('text', { x: nd.x, y: nd.y });
      t.textContent = nd.l;
      g.appendChild(t);
      svg.appendChild(g);
    });
    svg.setAttribute('aria-label', spec.aria);
    syncGateCopy();
    setTone('');
    statusEl.textContent = '已切换到' + spec.name + '。点「跑一次」开始。';
  }

  function syncGateCopy() {
    var spec = TOPO[state.topo];
    if (gateDesc) gateDesc.textContent = state.gate ? spec.gate : '无闸门 · 缺陷会随产物静默传播';
    if (hintEl) {
      hintEl.textContent = state.gate
        ? '有闸门：未被发现的缺陷变成一次可重试的显式失败。'
        : '无闸门：失败不会被检查，缺陷随产物继续走。';
    }
  }

  function syncChips() {
    chips.forEach(function (c) {
      var on = c.getAttribute('data-topo') === state.topo &&
        (c.getAttribute('data-gate') === '1') === state.gate;
      c.classList.toggle('is-active', on);
    });
  }

  function select(name, chip, resetLog) {
    if (!TOPO[name]) name = 'solo';
    state.topo = name;
    tabs.forEach(function (t) {
      var on = t.getAttribute('data-topo') === name;
      t.classList.toggle('is-active', on);
      t.setAttribute('aria-selected', String(on));
    });
    syncChips();
    if (chip) {
      chips.forEach(function (c) { c.classList.toggle('is-active', c === chip); });
    }
    if (resetLog && logEl) {
      while (logEl.firstChild) logEl.removeChild(logEl.firstChild);
    }
    render();
  }

  function run() {
    if (state.running) return;
    state.running = true;
    runBtn.disabled = true;
    clearMarks();
    var spec = TOPO[state.topo];
    var nodes = svg.querySelectorAll('.tac__node');
    var edges = svg.querySelectorAll('.tac__edge');
    for (var e = 0; e < edges.length; e++) edges[e].classList.add('is-flow');

    // 固定种子的线性同余：同一台机器、同一顺序，结果可复现。
    state.seed = (state.seed * 1664525 + 1013904223) >>> 0;
    function roll(salt) {
      var x = (state.seed + salt) >>> 0;
      x = (x * 1664525 + 1013904223) >>> 0;
      return x / 4294967296;
    }
    var trial = String((state.seed % 900) + 100);
    var willFail = roll(7) < 0.22; // 演示用先验，不是实测概率
    var failIdx = Math.floor(roll(13) * spec.nodes.length);
    var caught = state.gate && roll(29) < spec.detect;
    var i = 0;

    function step() {
      nodes[i].classList.add('is-run');
      if (willFail && i === failIdx) {
        nodes[i].classList.remove('is-run');
        nodes[i].classList.add(caught ? 'is-caught' : 'is-fail');
        var label = spec.nodes[i].l;
        setTone(caught ? 'warn' : 'bad');
        statusEl.textContent = '第 ' + trial + ' 次 · 节点 ' + label +
          (caught ? ' 失败，被闸门拦下' : ' 失败，缺陷继续传播');
        line('trial ' + trial + ' · node ' + label +
          (caught ? ' fail → gate caught → retry' : ' fail → no gate → propagate'),
          caught ? 'warn' : 'bad');
        finish();
        return;
      }
      i++;
      if (i >= nodes.length) {
        setTone('ok');
        statusEl.textContent = '第 ' + trial + ' 次 · ' + nodes.length + ' 个节点全部通过';
        line('trial ' + trial + ' · all ' + nodes.length + ' nodes pass', 'ok');
        finish();
        return;
      }
      window.setTimeout(step, reduce ? 0 : 170);
    }
    function finish() {
      state.running = false;
      runBtn.disabled = false;
    }
    window.setTimeout(step, reduce ? 0 : 110);
  }

  tabs.forEach(function (tab) {
    tab.addEventListener('click', function () {
      if (state.running) return;
      select(tab.getAttribute('data-topo'), null, true);
    });
    tab.addEventListener('keydown', function (event) {
      var delta = event.key === 'ArrowRight' ? 1 : (event.key === 'ArrowLeft' ? -1 : 0);
      if (!delta) return;
      event.preventDefault();
      var idx = tabs.indexOf(tab);
      var next = tabs[(idx + delta + tabs.length) % tabs.length];
      next.focus();
      if (!state.running) select(next.getAttribute('data-topo'), null, true);
    });
  });

  chips.forEach(function (chip) {
    chip.addEventListener('click', function () {
      if (state.running) return;
      state.gate = chip.getAttribute('data-gate') === '1';
      gateEl.checked = state.gate;
      select(chip.getAttribute('data-topo'), chip, true);
    });
  });

  gateEl.addEventListener('change', function () {
    if (state.running) { gateEl.checked = !gateEl.checked; return; }
    state.gate = gateEl.checked;
    syncGateCopy();
    syncChips();
    setTone('');
    statusEl.textContent = state.gate ? '闸门已打开。点「跑一次」看它怎么拦。' : '闸门已关闭。点「跑一次」看缺陷怎么溜过去。';
  });

  runBtn.addEventListener('click', run);
  render();
})();

/* ── 证据模式（2026-10-31）──────────────────────────────────────────────────
   页面上的 9 组台账数字各挂一个 data-proof 键；按钮打开的是一个由 DOM 现场
   取值生成的证据表。值不写死在 JS 里 —— 否则页面数字更新、面板还停在旧值，
   正是这个站最反对的「两份真相」。 */
(function () {
  'use strict';
  var toggle = document.getElementById('proofToggle');
  var panel = document.getElementById('proofPanel');
  var bar = document.querySelector('[data-proof-bar]');
  var list = document.querySelector('[data-proof-list]');
  var countEl = document.querySelector('[data-proof-count]');
  if (!toggle || !panel || !list) return;

  var META = {
    views: { t: '控制台视图', cmd: 'ls webui/src/views/*.vue | wc -l', src: 'webui/src/views/' },
    'core-modules': { t: '内核子系统模块', cmd: 'ls -d core/src/*/ | wc -l', src: 'core/src/' },
    tools: { t: '内置工具', cmd: 'agent.selfcheck · tools.count', src: 'core/src/tools/' },
    rpc: { t: 'RPC 方法声明', cmd: 'npm run typecheck', src: 'core/src/gateway/protocol.rs' },
    'core-tests': { t: '核心测试通过', cmd: 'cargo test -p marguerite-core --lib', src: 'README.md 质量闸门表' },
    'webui-tests': { t: '控制台单测通过', cmd: 'npm run test:unit', src: 'README.md 质量闸门表' },
    gate: { t: '全量门禁检查项', cmd: 'bash scripts/verify.sh --clean --all', src: 'scripts/verify.sh' },
    'cli-routes': { t: 'CLI 路由', cmd: 'grep -c "route_table.register" core/src/cli/program.rs', src: 'core/src/cli/program.rs' },
    'why-chain': { t: '5-Why 追问层数', cmd: '每层一个机制 + 一条可伪证条件', src: '#compare .why' }
  };

  function first(key) {
    return document.querySelector('[data-proof="' + key + '"]');
  }
  function valueOf(el) {
    var n = el.querySelector('.stat__n');
    if (n) {
      // 计数动画会把 textContent 暂时写成中间值；台账要读目标值，不能读动画帧。
      var target = n.querySelector('[data-count]');
      if (target) return (target.getAttribute('data-count') || '').trim();
      return n.textContent.replace(/\s+/g, ' ').trim();
    }
    var steps = el.querySelectorAll('.why__step');
    if (steps.length) return steps.length + ' 层';
    return (el.getAttribute('data-proof-value') || '').trim();
  }
  function build() {
    while (list.firstChild) list.removeChild(list.firstChild);
    var count = 0;
    Object.keys(META).forEach(function (key) {
      var el = first(key);
      if (!el) return;
      var v = valueOf(el);
      if (!v) return;
      count++;
      var row = document.createElement('div');
      row.className = 'proof__row';
      var t = document.createElement('p'); t.className = 'proof__rt'; t.textContent = META[key].t;
      var val = document.createElement('p'); val.className = 'proof__rv'; val.textContent = v;
      var cmd = document.createElement('p'); cmd.className = 'proof__rc'; cmd.textContent = '$ ' + META[key].cmd;
      var src = document.createElement('p'); src.className = 'proof__rs'; src.textContent = '来源：' + META[key].src;
      var go = document.createElement('button');
      go.type = 'button';
      go.className = 'proof__go';
      go.setAttribute('data-proof-go', key);
      go.textContent = '定位';
      row.appendChild(t); row.appendChild(val); row.appendChild(cmd); row.appendChild(src); row.appendChild(go);
      list.appendChild(row);
    });
    return count;
  }
  function open() {
    build();
    if (typeof panel.showModal === 'function') {
      if (!panel.open) panel.showModal();
    } else {
      panel.setAttribute('open', '');
    }
  }
  function close() {
    if (typeof panel.close === 'function') {
      if (panel.open) panel.close();
    } else {
      panel.removeAttribute('open');
    }
  }
  function setOn(on) {
    toggle.setAttribute('aria-pressed', String(on));
    document.body.classList.toggle('proof-on', on);
    var count = build();
    if (countEl) countEl.textContent = String(count);
    if (bar) bar.hidden = !on;
    if (on) open(); else close();
  }

  toggle.addEventListener('click', function () {
    setOn(toggle.getAttribute('aria-pressed') !== 'true');
  });
  var closeBtn = document.querySelector('[data-proof-close]');
  if (closeBtn) closeBtn.addEventListener('click', close);
  var openBtn = document.querySelector('[data-proof-open]');
  if (openBtn) openBtn.addEventListener('click', open);
  panel.addEventListener('click', function (event) {
    if (event.target === panel) close();
  });
  document.addEventListener('keydown', function (event) {
    if (event.key === 'Escape' && panel.open) close();
  });
  list.addEventListener('click', function (event) {
    var btn = event.target.closest ? event.target.closest('[data-proof-go]') : null;
    if (!btn) return;
    var el = first(btn.getAttribute('data-proof-go'));
    if (!el) return;
    close();
    el.scrollIntoView({ behavior: reduceMotion() ? 'auto' : 'smooth', block: 'center' });
    el.classList.add('is-flash');
    window.setTimeout(function () { el.classList.remove('is-flash'); }, 1000);
  });
  function reduceMotion() {
    return window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  }
})();
