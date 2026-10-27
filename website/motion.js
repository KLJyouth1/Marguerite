/* ============================================================================
   Marguerite 官网 · 动效层（零构建，先于 main.js 载入）
   职责：滚动揭示 / 吸顶导航 / 鼠标视差。禁用 JS 时版面完整，降级时只去动效。
   ========================================================================== */
(function () {
  'use strict';

  var doc = document;
  var root = doc.documentElement;
  var reduceMQ = window.matchMedia('(prefers-reduced-motion: reduce)');
  var fineMQ = window.matchMedia('(pointer: fine)');
  function motionOn() { return !reduceMQ.matches; }

  /* 1) 滚动揭示 —— opacity + translateY 组合入场，缓动无弹跳 */
  if (motionOn() && 'IntersectionObserver' in window) {
    doc.body.classList.add('motion');
    var io = new IntersectionObserver(function (entries) {
      entries.forEach(function (e) {
        if (e.isIntersecting) { e.target.classList.add('in'); io.unobserve(e.target); }
      });
    }, { threshold: 0.01, rootMargin: '0px 0px -10% 0px' });

    // 同父级内的 reveal 元素错峰
    var reveals = doc.querySelectorAll('.reveal-up');
    for (var i = 0; i < reveals.length; i++) {
      var el = reveals[i];
      var sibs = el.parentNode ? el.parentNode.children : [];
      var idx = 0;
      for (var k = 0; k < sibs.length; k++) {
        if (sibs[k] === el) break;
        if (sibs[k].classList && sibs[k].classList.contains('reveal-up')) idx++;
      }
      el.style.transitionDelay = (idx * 80) + 'ms';
      io.observe(el);
    }
  }

  /* 2) 吸顶导航 + 阅读进度 + 当前章节高亮，共用一个滚动监听，rAF 节流 */
  var nav = doc.getElementById('nav');
  var aura = doc.querySelector('.aura');
  var heroInner = doc.querySelector('.hero__inner');
  var heroKicker = doc.querySelector('.hero__kicker');
  var barFill = doc.querySelector('[data-scrollbar]');
  var currentables = [];
  Array.prototype.forEach.call(doc.querySelectorAll('.nav__links a'), function (link) {
    var id = (link.getAttribute('href') || '').replace('#', '');
    var sec = id ? doc.getElementById(id) : null;
    if (sec) currentables.push({ link: link, el: sec });
  });

  var ticking = false;
  function onScroll() {
    if (ticking) return;
    ticking = true;
    requestAnimationFrame(function () {
      var y = window.pageYOffset || root.scrollTop || 0;
      if (nav) { if (y > 8) nav.classList.add('is-stuck'); else nav.classList.remove('is-stuck'); }
      if (barFill) {
        var max = Math.max(1, (doc.documentElement.scrollHeight || 0) - window.innerHeight);
        barFill.style.transform = 'scaleX(' + Math.min(1, y / max).toFixed(4) + ')';
      }
      // 以「最后一个越过导航下沿的章节」作为当前位置
      var current = null;
      for (var i = 0; i < currentables.length; i++) {
        if (currentables[i].el.getBoundingClientRect().top <= 140) current = currentables[i];
      }
      for (var k = 0; k < currentables.length; k++) {
        currentables[k].link.classList.toggle('is-current', y > 40 && currentables[k] === current);
      }
      ticking = false;
    });
  }
  window.addEventListener('scroll', onScroll, { passive: true });
  window.addEventListener('resize', onScroll, { passive: true });
  onScroll();

  /* 鼠标视差：仅桌面、pointer:fine、且未开启 reduced-motion */
  if (motionOn() && fineMQ.matches && aura) {
    var tx = 0, ty = 0, pending = false;
    window.addEventListener('mousemove', function (e) {
      var cx = window.innerWidth / 2, cy = window.innerHeight / 2;
      // 归一化到 [-1,1]
      tx = (e.clientX - cx) / cx;
      ty = (e.clientY - cy) / cy;
      if (!pending) {
        pending = true;
        requestAnimationFrame(function () {
          pending = false;
          // 背景辉光反向轻移，制造景深；幅度克制（≤18px）
          aura.style.transform = 'translate3d(' + (tx * -18) + 'px,' + (ty * -18) + 'px,0)';
          if (heroInner) heroInner.style.transform = 'translate3d(' + (tx * 6) + 'px,' + (ty * 4) + 'px,0)';
        });
      }
    }, { passive: true });
  }

  /* 4) 真实界面滚动漫推：视口内从 1.06 收到 1.0（只改 transform） */
  if (motionOn()) {
    var zooms = Array.prototype.slice.call(doc.querySelectorAll('.zoom-shot img'));
    if (zooms.length) {
      var zoomTicking = false;
      var updateZooms = function () {
        zoomTicking = false;
        var vh = window.innerHeight || 1;
        zooms.forEach(function (img) {
          var rect = img.getBoundingClientRect();
          if (rect.bottom < -80 || rect.top > vh + 80) return;
          var progress = (vh - rect.top) / (vh + rect.height);
          var clamped = Math.max(0, Math.min(1, progress));
          var scale = 1.06 - clamped * 0.06;
          img.style.setProperty('--z', scale.toFixed(4));
        });
      };
      var onZoomScroll = function () {
        if (zoomTicking) return;
        zoomTicking = true;
        window.requestAnimationFrame(updateZooms);
      };
      window.addEventListener('scroll', onZoomScroll, { passive: true });
      window.addEventListener('resize', onZoomScroll, { passive: true });
      updateZooms();
    }
  }

  /* 动态监听 reduced-motion 切换：降级时撤掉 JS 揭示态与毛玻璃 */
  if ('addEventListener' in reduceMQ) {
    reduceMQ.addEventListener('change', function () {
      if (reduceMQ.matches) {
        doc.body.classList.remove('motion');
        if (nav) nav.classList.remove('is-stuck');
        if (aura) aura.style.transform = '';
        if (heroInner) heroInner.style.transform = '';
      }
    });
  }
})();
