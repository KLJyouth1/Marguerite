/* ============================================================================
   Marguerite 官网 · 渐进增强脚本
   本站在禁用 JS 时功能完整。本文件只补两件事：移动端菜单开合、命令复制按钮。
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
})();
