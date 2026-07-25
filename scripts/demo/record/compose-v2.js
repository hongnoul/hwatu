(() => {
  'use strict';

  const DURATION = 21;
  const params = new URLSearchParams(location.search);
  const fixedTime = params.has('t') ? Math.max(0, Math.min(DURATION, Number(params.get('t')) || 0)) : null;
  const evidenceDir = params.get('evidence') || './evidence-v2';
  const fixture = {
    scores: [85.13, 93.42, 98.79],
    motion: { durationMs: 400, easing: 'cubic-bezier(0.4, 0, 0.2, 1)' },
    handoff: { sessionId: 2, tab: 3, scroll: 68, frame: 12 },
    proof: '13 ms windows · 87 ms verification · one binary'
  };
  let evidence = fixture;

  const $ = (selector) => document.querySelector(selector);
  const $$ = (selector) => [...document.querySelectorAll(selector)];
  const clamp = (value, min = 0, max = 1) => Math.max(min, Math.min(max, value));
  const between = (time, start, end) => clamp((time - start) / (end - start));
  const smooth = (value) => value * value * (3 - 2 * value);

  async function loadEvidence() {
    try {
      const response = await fetch(`${evidenceDir.replace(/\/$/, '')}/manifest.json`, { cache: 'no-store' });
      if (!response.ok) throw new Error(String(response.status));
      const loaded = await response.json();
      evidence = {
        ...fixture,
        ...loaded,
        motion: { ...fixture.motion, ...loaded.motion },
        handoff: { ...fixture.handoff, ...loaded.handoff },
        scores: Array.isArray(loaded.scores) && loaded.scores.length >= 3 ? loaded.scores.slice(0, 3) : fixture.scores
      };
      $('#evidenceStatus').textContent = 'EVIDENCE · MANIFEST';
    } catch (_) {
      $('#evidenceStatus').textContent = 'EVIDENCE · DEMO FIXTURE';
      $('#evidenceStatus').style.color = 'var(--amber)';
    }
    $('#proofLine').textContent = evidence.proof;
    render(fixedTime ?? 0);
  }

  function showScene(id, index) {
    $$('.scene').forEach((scene) => scene.classList.toggle('active', scene.id === id));
    $('.topline').style.opacity = id === 'end' ? '0' : '1';
    $('#sceneIndex').textContent = `${String(index).padStart(2, '0')} / 03`;
  }

  function renderMeasure(time) {
    showScene('measure', 1);
    const progress = smooth(between(time, 0.4, 6.6));
    const checkpoint = progress < .36 ? 0 : progress < .72 ? 1 : 2;
    const local = checkpoint === 0 ? progress / .36 : checkpoint === 1 ? (progress - .36) / .36 : (progress - .72) / .28;
    const scoreA = evidence.scores[checkpoint];
    const scoreB = evidence.scores[Math.min(2, checkpoint + 1)];
    const score = checkpoint === 2 ? scoreA : scoreA + (scoreB - scoreA) * clamp(local);
    const error = 1 - progress * .92;
    $('.build-art').style.setProperty('--error', error.toFixed(3));
    $('#checkpointLabel').textContent = `CHECKPOINT 0${checkpoint + 1}`;
    $('#railTitle').textContent = 'VISUAL MATCH';
    $('#railLeft').textContent = `${evidence.scores[0].toFixed(2)}%`;
    $('#railRight').textContent = `${score.toFixed(2)}%`;
    $('#railFill').style.cssText = `width:${progress * 100}%;background:var(--blue)`;
    $('#railPin').style.display = 'none';
  }

  function motionPosition(local) {
    if (local < .3) return between(local, 0, .3) * .5;
    if (local < .55) return .5 + between(local, .3, .55) * .3;
    if (local < .78) return .8 - between(local, .55, .78) * .3;
    return .5;
  }

  function renderMotion(time) {
    showScene('motion', 2);
    const local = between(time, 7, 13.2);
    const position = smooth(motionPosition(local));
    const x = 2.5 + position * 18;
    const y = 3.5 + Math.sin(position * Math.PI) * 7;
    const rotation = -8 + position * 16;
    $$('.moving-card').forEach((card) => {
      card.style.setProperty('--motion-x', `${x}cqw`);
      card.style.setProperty('--motion-y', `${y}cqw`);
      card.style.setProperty('--motion-r', `${rotation}deg`);
    });
    const frame = Math.round(position * 24);
    $$('.frame-readout').forEach((label) => { label.textContent = `FRAME ${String(frame).padStart(2, '0')}`; });
    $('#ghostBefore').style.left = `${Math.max(0, position * 100 - 18)}%`;
    $('#ghostNow').style.left = `${position * 100}%`;
    $('#directionBefore').style.opacity = local < .6 ? '1' : '.25';
    $('#directionAfter').style.opacity = local >= .5 ? '1' : '.25';
    $('#repeatProof').textContent = local > .78 ? 'REPEAT · EXACT FRAME' : `${evidence.motion.durationMs} MS · ${evidence.motion.easing}`;
    $('#railTitle').textContent = 'ANIMATION TIME';
    $('#railLeft').textContent = '0%';
    $('#railRight').textContent = `t = ${Math.round(position * 100)}%`;
    $('#railFill').style.cssText = `width:${position * 100}%;background:var(--amber)`;
    $('#railPin').style.cssText = `display:block;left:${position * 100}%`;
  }

  function renderHandoff(time) {
    showScene('handoff', 3);
    const reveal = smooth(between(time, 14.5, 17.1));
    const action = between(time, 13.4, 14.8);
    $('.handoff-action').style.opacity = String(action < .15 || action > .95 ? 0 : Math.sin(action * Math.PI));
    $('.live-browser').style.transform = `translateX(${(1 - reveal) * 110}%)`;
    $('.live-browser').style.opacity = String(reveal);
    $('.state-chip').style.opacity = String(1 - reveal * .72);
    const h = evidence.handoff;
    $('.session-badge').textContent = `HEADLESS SESSION · ID ${h.sessionId}`;
    $('.state-chip span').textContent = `scroll ${h.scroll}% · tab ${h.tab} · frame ${h.frame}`;
    $('.preserved-tag').textContent = `✓ SAME TAB · SCROLL ${h.scroll}% · FRAME ${h.frame}`;
    $('.scroll-indicator i').style.top = `${h.scroll}%`;
    $('.scroll-indicator span').style.top = `${h.scroll}%`;
    $('.scroll-indicator span').textContent = `${h.scroll}%`;
    $('#railTitle').textContent = 'SESSION STATE';
    $('#railLeft').textContent = reveal < .5 ? 'HEADLESS' : 'PRESERVED';
    $('#railRight').textContent = reveal < .98 ? '→ LIVE' : 'LIVE · READY';
    $('#railFill').style.cssText = `width:${reveal * 100}%;background:var(--mint)`;
    $('#railPin').style.display = 'none';
  }

  function renderEnd() {
    showScene('end', 3);
    $('#railTitle').textContent = 'ONE WORKFLOW';
    $('#railLeft').textContent = 'MEASURE';
    $('#railRight').textContent = 'HAND OFF';
    $('#railFill').style.cssText = 'width:100%;background:var(--mint)';
    $('#railPin').style.display = 'none';
  }

  function render(rawTime) {
    const time = Math.max(0, Math.min(DURATION, rawTime));
    $('#railClock').textContent = `${time.toFixed(1).padStart(4, '0')} / ${DURATION.toFixed(1)}`;
    if (time < 7) renderMeasure(time);
    else if (time < 13.3) renderMotion(time);
    else if (time < 19.1) renderHandoff(time);
    else renderEnd(time);
    document.documentElement.dataset.time = time.toFixed(3);
  }

  let started = performance.now();
  function tick(now) {
    const elapsed = ((now - started) / 1000) % DURATION;
    render(elapsed);
    requestAnimationFrame(tick);
  }

  loadEvidence().then(() => {
    if (fixedTime === null) requestAnimationFrame(tick);
    else render(fixedTime);
  });
})();
