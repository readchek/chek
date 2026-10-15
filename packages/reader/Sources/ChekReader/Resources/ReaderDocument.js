(() => {
  const world = window;
  let token = null;
  let lastAnchor = null;
  let restoring = true;
  let captureTimer = null;
  let resizeTimer = null;
  let topInset = 0;
  let bottomInset = 0;
  let baseline = null;
  let scale = 1;
  let operation = 0;
  let fixedLayout = false;
  let lastPublished = '';
  let lastInteraction = 0;
  let touch = null;
  const frames = () => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
  const resourceHref = () => window.location.pathname.slice(1);
  const validText = node => node && node.nodeType === Node.TEXT_NODE && /\S/.test(node.data)
    && !node.parentElement?.closest('script,style,noscript,template');

  function install() {
    if (!document.head) return false;
    if (!document.querySelector('meta[name="viewport"]')) {
      const meta = document.createElementNS('http://www.w3.org/1999/xhtml', 'meta');
      meta.setAttribute('name', 'viewport');
      meta.setAttribute('content', 'width=device-width, initial-scale=1');
      meta.setAttribute('data-chek-reader-viewport', '');
      document.head.appendChild(meta);
    }
    if (!document.querySelector('style[data-chek-reader-insets]')) {
      const style = document.createElementNS('http://www.w3.org/1999/xhtml', 'style');
      style.setAttribute('data-chek-reader-insets', '');
      style.textContent = 'html[data-chek-reader-reflow] { padding-inline: 1rem; padding-block-end: max(1rem, var(--chek-bottom-inset, 0px)); box-sizing: border-box; -webkit-text-size-adjust: 100%; }';
      document.head.appendChild(style);
    }
    return true;
  }

  if (!install()) {
    const observer = new MutationObserver(() => { if (install()) observer.disconnect(); });
    observer.observe(document, {childList: true, subtree: true});
    document.addEventListener('DOMContentLoaded', () => { observer.disconnect(); install(); }, {once: true});
  }

  function pathTo(node) {
    const path = [];
    while (node && node !== document.body && path.length <= 128) {
      const parent = node.parentNode;
      if (!parent) return [];
      path.unshift(Array.prototype.indexOf.call(parent.childNodes, node));
      node = parent;
    }
    return node === document.body ? path : [];
  }

  function textAtTop() {
    const y = Math.max(1, Math.min(innerHeight - 1, topInset + 12));
    const bottom = Math.max(y, innerHeight - bottomInset - 1);
    const roots = new Set();
    for (const sampleY of [y, Math.min(bottom, y + 24), Math.min(bottom, y + 64)]) {
      for (const x of [24, innerWidth / 2, Math.max(24, innerWidth - 24)]) {
        const range = document.caretRangeFromPoint?.(x, sampleY);
        if (range && validText(range.startContainer)) {
          return {node: range.startContainer, offset: range.startOffset};
        }
        const root = document.elementFromPoint(x, sampleY);
        if (root && root !== document.body && root !== document.documentElement) roots.add(root);
      }
    }
    // A blank line or illustration must not trigger a chapter-wide geometry
    // scan. Search only hit-tested visible subtrees, within one shared budget.
    const deadline = performance.now() + 4;
    let visited = 0;
    for (const root of roots) {
      const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
      let node;
      while ((node = walker.nextNode()) && ++visited <= 128 && performance.now() < deadline) {
        if (!validText(node)) continue;
        const range = document.createRange();
        range.selectNodeContents(node);
        const rect = range.getBoundingClientRect();
        if (rect.bottom > y && rect.top < bottom && rect.width > 0) return {node, offset: 0};
      }
      if (visited >= 128 || performance.now() >= deadline) break;
    }
    return null;
  }

  function capture() {
    if (!document.body && !fixedLayout) return null;
    const point = fixedLayout ? null : textAtTop();
    const span = fixedLayout ? 0 : Math.max(0, document.documentElement.scrollHeight - innerHeight);
    const location = {
      version: 1, resourceHref: resourceHref(), nodePath: [], textOffset: 0,
      quote: '', prefix: '', suffix: '', progression: span > 0 ? Math.max(0, Math.min(1, scrollY / span)) : 0
    };
    if (point) {
      const text = point.node.data;
      let offset = Math.min(point.offset, Math.max(0, text.length - 1));
      if (isLowSurrogate(text.charCodeAt(offset)) && isHighSurrogate(text.charCodeAt(offset - 1))) offset--;
      location.nodePath = pathTo(point.node);
      location.textOffset = offset;
      location.quote = scalarSlice(text, offset, offset + 64);
      location.prefix = scalarSlice(text, Math.max(0, offset - 32), offset);
      const suffixStart = offset + location.quote.length;
      location.suffix = scalarSlice(text, suffixStart, suffixStart + 32);
    }
    return location;
  }

  function isHighSurrogate(value) { return value >= 0xD800 && value <= 0xDBFF; }
  function isLowSurrogate(value) { return value >= 0xDC00 && value <= 0xDFFF; }
  function scalarSlice(text, start, end) {
    end = Math.min(end, text.length);
    if (isLowSurrogate(text.charCodeAt(start)) && isHighSurrogate(text.charCodeAt(start - 1))) start++;
    if (isHighSurrogate(text.charCodeAt(end - 1)) && isLowSurrogate(text.charCodeAt(end))) end--;
    return text.slice(start, end);
  }

  function publish() {
    if (restoring || !token) return null;
    const started = performance.now();
    const location = capture();
    if (location) {
      lastAnchor = location;
      const encoded = JSON.stringify(location);
      if (encoded !== lastPublished) {
        lastPublished = encoded;
        window.webkit?.messageHandlers?.readingLocation?.postMessage({
          token, location, captureMilliseconds: performance.now() - started
        });
      }
    }
    return location;
  }

  function resolve(anchor) {
    if (anchor.nodePath?.length) {
      let node = document.body;
      for (const index of anchor.nodePath) node = node?.childNodes[index];
      if (validText(node) && anchor.textOffset <= node.length
        && node.data.slice(anchor.textOffset, anchor.textOffset + anchor.quote.length) === anchor.quote) {
        return {node, offset: anchor.textOffset};
      }
    }
    if (!anchor.quote) return null;
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    let node, visited = 0, fallback = null;
    while ((node = walker.nextNode()) && ++visited <= 20000) {
      if (!validText(node)) continue;
      let from = 0, offset, occurrences = 0;
      while ((offset = node.data.indexOf(anchor.quote, from)) !== -1 && occurrences++ < 128) {
        const point = {node, offset};
        if (!fallback) fallback = point;
        const prefix = node.data.slice(Math.max(0, offset - anchor.prefix.length), offset);
        const suffix = node.data.slice(offset + anchor.quote.length, offset + anchor.quote.length + anchor.suffix.length);
        if (prefix === anchor.prefix && suffix === anchor.suffix) return point;
        from = offset + 1;
      }
    }
    return fallback;
  }

  function scrollToAnchor(anchor) {
    if (!anchor || fixedLayout || anchor.resourceHref !== resourceHref()) return;
    const point = resolve(anchor);
    if (point) {
      const range = document.createRange();
      range.setStart(point.node, point.offset);
      range.setEnd(point.node, Math.min(point.offset + 1, point.node.length));
      const rect = range.getBoundingClientRect();
      if (rect.height > 0) { window.scrollTo(0, scrollY + rect.top - topInset - 12); return; }
    }
    window.scrollTo(0, Math.max(0, document.documentElement.scrollHeight - innerHeight) * anchor.progression);
  }

  async function settled() {
    const cleanup = [];
    const pending = [];
    for (const image of document.images ?? []) {
      if (pending.length >= 2000) break;
      if (image.complete) continue;
      pending.push(new Promise(resolve => {
        const finished = () => { remove(); resolve(); };
        const remove = () => {
          image.removeEventListener('load', finished);
          image.removeEventListener('error', finished);
        };
        image.addEventListener('load', finished, {once: true});
        image.addEventListener('error', finished, {once: true});
        cleanup.push(remove);
      }));
    }
    // Fonts and images load together. Share a single timeout and release all
    // event listeners even when an image never completes.
    let timeout;
    try {
      await Promise.race([
        Promise.all([document.fonts?.ready, ...pending]),
        new Promise(resolve => { timeout = setTimeout(resolve, 2500); })
      ]);
    } finally {
      clearTimeout(timeout);
      for (const remove of cleanup) remove();
    }
    await frames();
  }

  function applyScale(nextScale) {
    if (!document.body) return;
    if (!baseline) baseline = {
      root: parseFloat(getComputedStyle(document.documentElement).fontSize),
      body: parseFloat(getComputedStyle(document.body).fontSize),
      rootInline: document.documentElement.style.getPropertyValue('font-size'),
      rootPriority: document.documentElement.style.getPropertyPriority('font-size'),
      bodyInline: document.body.style.getPropertyValue('font-size'),
      bodyPriority: document.body.style.getPropertyPriority('font-size')
    };
    scale = Math.max(0.75, Math.min(2, nextScale));
    if (scale === 1) {
      document.documentElement.style.setProperty('font-size', baseline.rootInline, baseline.rootPriority);
      document.body.style.setProperty('font-size', baseline.bodyInline, baseline.bodyPriority);
    } else {
      document.documentElement.style.setProperty('font-size', `${baseline.root * scale}px`, 'important');
      document.body.style.setProperty('font-size', `${baseline.body * scale}px`, 'important');
    }
  }

  function setInsets(top, bottom) {
    topInset = fixedLayout ? 0 : Math.max(0, Math.min(300, top));
    bottomInset = fixedLayout ? 0 : Math.max(0, Math.min(300, bottom));
    document.documentElement.style.setProperty('--chek-bottom-inset', `${bottomInset}px`);
  }

  function documentError() {
    if (!['application/xhtml+xml', 'image/svg+xml', 'application/xml', 'text/xml'].includes(document.contentType)) return null;
    // WebKit's XMLErrors inserts this exact report before the root's first
    // child (or in a generated body for missing/SVG roots). An authored element
    // merely named parsererror elsewhere in a book is not a parse failure.
    // https://github.com/WebKit/WebKit/blob/main/Source/WebCore/xml/XMLErrors.cpp
    const xhtml = 'http://www.w3.org/1999/xhtml';
    for (const parent of [document.documentElement, document.body]) {
      const report = parent?.firstChild;
      if (!(report instanceof Element) || report.localName !== 'parsererror'
        || report.namespaceURI !== xhtml || report.style.display !== 'block'
        || report.style.whiteSpace !== 'pre') continue;
      const children = Array.from(report.children);
      if (children.length !== 3 || children.some(child => child.namespaceURI !== xhtml)
        || children[0].localName !== 'h3' || children[1].localName !== 'div'
        || children[2].localName !== 'h3'
        || children[0].textContent !== 'This page contains the following errors:'
        || children[2].textContent !== 'Below is a rendering of the page up to the first error.'
        || children[1].style.fontFamily !== 'monospace'
        || children[1].style.fontSize !== '12px') continue;
      return children[1].textContent.slice(0, 2048);
    }
    return null;
  }

  world.chekReader = {
    async activate(nextToken, anchor, nextScale, top, bottom, isFixedLayout) {
      const failure = documentError();
      if (failure !== null) return {documentError: failure};
      const generation = ++operation;
      restoring = true;
      token = nextToken;
      lastPublished = '';
      fixedLayout = isFixedLayout;
      document.documentElement.toggleAttribute('data-chek-reader-reflow', !fixedLayout);
      if (fixedLayout) document.querySelector('meta[data-chek-reader-viewport]')?.remove();
      clearTimeout(captureTimer);
      setInsets(top, bottom);
      await settled();
      if (generation !== operation) return null;
      if (!fixedLayout) applyScale(nextScale);
      await frames();
      if (generation !== operation) return null;
      if (anchor) {
        lastAnchor = anchor;
        scrollToAnchor(anchor);
        await frames();
        if (generation !== operation) return null;
        scrollToAnchor(anchor);
      }
      restoring = false;
      return publish();
    },
    capture() { return restoring ? null : publish(); },
    async setScale(nextScale) {
      if (fixedLayout) return publish();
      const anchor = restoring ? lastAnchor : capture();
      const generation = ++operation;
      restoring = true;
      clearTimeout(captureTimer);
      applyScale(nextScale);
      await settled();
      if (generation !== operation) return null;
      scrollToAnchor(anchor);
      await frames();
      if (generation !== operation) return null;
      restoring = false;
      return publish();
    },
    setInsets
  };

  window.addEventListener('scroll', () => {
    clearTimeout(captureTimer);
    if (!restoring) {
      captureTimer = setTimeout(publish, 350);
      interact('active', true);
    }
  }, {passive: true});
  window.addEventListener('scrollend', () => {
    clearTimeout(captureTimer);
    if (!restoring) captureTimer = setTimeout(publish, 80);
  }, {passive: true});

  function interact(kind, throttled = false) {
    if (!token || restoring) return;
    const now = performance.now();
    if (throttled && now - lastInteraction < 1000) return;
    lastInteraction = now;
    window.webkit?.messageHandlers?.readerInteraction?.postMessage({token, kind});
  }

  const hasSelection = () => !window.getSelection()?.isCollapsed;
  const interactiveTarget = target => target instanceof Element
    && target.closest('a,button,input,textarea,select,[contenteditable]');
  window.addEventListener('touchstart', event => {
    interact('active');
    const point = event.touches[0];
    touch = event.touches.length === 1 && !hasSelection() && !interactiveTarget(event.target)
      && (window.visualViewport?.scale ?? 1) <= 1.01
      ? {x: point.clientX, y: point.clientY, time: performance.now()} : null;
  }, {passive: true});
  window.addEventListener('touchmove', event => {
    if (event.touches.length !== 1) touch = null;
  }, {passive: true});
  window.addEventListener('touchcancel', () => { touch = null; }, {passive: true});
  window.addEventListener('touchend', event => {
    const start = touch;
    touch = null;
    if (!start || event.touches.length || hasSelection() || interactiveTarget(event.target)) return;
    const point = event.changedTouches[0];
    if (!point) return;
    const x = point.clientX - start.x, y = point.clientY - start.y;
    const duration = performance.now() - start.time;
    if (duration < 350 && Math.hypot(x, y) < 10) interact('tap');
    else if (duration < 900 && Math.abs(x) >= 70 && Math.abs(x) > Math.abs(y) * 1.5
      && (window.visualViewport?.scale ?? 1) <= 1.01) {
      interact(x < 0 ? 'next' : 'previous');
    }
  }, {passive: true});
  window.addEventListener('resize', () => {
    if (!token || !lastAnchor || fixedLayout) return;
    const anchor = lastAnchor;
    const generation = ++operation;
    restoring = true;
    clearTimeout(captureTimer);
    clearTimeout(resizeTimer);
    resizeTimer = setTimeout(async () => {
      await settled();
      if (generation !== operation) return;
      scrollToAnchor(anchor);
      await frames();
      if (generation !== operation) return;
      restoring = false;
      publish();
    }, 120);
  }, {passive: true});
})();
