/* ============================================================================
   Marguerite 官网 · 渐进增强脚本
   本站在禁用 JS 时功能完整。本文件只补四件事：移动端菜单、命令复制、
   顶部公告条的关闭记忆、首屏提问带进下面的真实演示。
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
})();
