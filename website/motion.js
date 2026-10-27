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

  /* 2) 吸顶导航（滚动后加毛玻璃）+ 3) 鼠标视差，共用一个滚动/鼠标监听，rAF 节流 */
  var nav = doc.getElementById('nav');
  var aura = doc.querySelector('.aura');
  var heroInner = doc.querySelector('.hero__inner');
  var heroKicker = doc.querySelector('.hero__kicker');

  var ticking = false;
  function onScroll() {
    if (ticking) return;
    ticking = true;
    requestAnimationFrame(function () {
      var y = window.pageYOffset || root.scrollTop || 0;
      if (nav) { if (y > 8) nav.classList.add('is-stuck'); else nav.classList.remove('is-stuck'); }
      ticking = false;
    });
  }
  window.addEventListener('scroll', onScroll, { passive: true });
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
