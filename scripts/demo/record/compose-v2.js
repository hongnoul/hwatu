(() => {
  'use strict';

  const DURATION = 21;
  const params = new URLSearchParams(location.search);
  const autoplay = params.get('autoplay') === '1';
  const fixedTime = params.has('t') && !autoplay
    ? Math.max(0, Math.min(DURATION, Number(params.get('t')) || 0))
    : null;
  const evidenceDir = params.get('evidence') || './evidence-v2';
  let evidence = null;

  const $ = (selector) => document.querySelector(selector);
  const $$ = (selector) => [...document.querySelectorAll(selector)];
  const clamp = (value, min = 0, max = 1) => Math.max(min, Math.min(max, value));
  const between = (time, start, end) => clamp((time - start) / (end - start));
  const smooth = (value) => value * value * (3 - 2 * value);

  function assetUrl(path) {
    if (!path) return '';
    return `${evidenceDir.replace(/\/$/, '')}/${String(path).replace(/^\.\//, '')}`;
  }

  function setBackground(selector, path) {
    const element = $(selector);
    if (element && path) element.style.backgroundImage = `url("${assetUrl(path).replace(/"/g, '%22')}")`;
  }

  function applyEvidence() {
    const assets = evidence.assets;
    setBackground('.reference-art', assets.reference);
    setBackground('.handoff-image', assets.handoff);
    $('#typedValue').textContent = `“${evidence.handoff.value}”`;
    $('#proofLine').textContent = evidence.proof.toUpperCase();
    document.documentElement.dataset.evidence = 'captured';
    $('#evidenceStatus').textContent = 'REAL WEBKIT · CAPTURED';
  }

  async function loadEvidence() {
    try {
      const response = await fetch(`${evidenceDir.replace(/\/$/, '')}/manifest.json`, { cache: 'no-store' });
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      const loaded = await response.json();
      const assets = loaded.assets || {};
      if (loaded.schema !== 'hwatu.demo.capture-v2/1'
          || !Array.isArray(loaded.scores) || loaded.scores.length !== 2
          || !Array.isArray(assets.builds) || assets.builds.length !== 2
          || !Array.isArray(assets.heatmaps) || assets.heatmaps.length !== 2
          || !Array.isArray(assets.motionReferenceFrames) || assets.motionReferenceFrames.length !== 4
          || !Array.isArray(assets.motionBuildFrames) || assets.motionBuildFrames.length !== 4
          || !loaded.handoff?.statePreserved) {
        throw new Error('incomplete evidence manifest');
      }
      evidence = loaded;
      applyEvidence();
    } catch (error) {
      document.documentElement.dataset.evidence = 'missing';
      $('#evidenceStatus').textContent = 'EVIDENCE MISSING';
      $('#evidenceStatus').style.color = 'var(--red)';
      console.error('hwatu demo evidence:', error);
    }
  }

  function showScene(id, verb) {
    $$('.scene').forEach((scene) => scene.classList.toggle('active', scene.id === id));
    $('.topline').style.opacity = id === 'end' ? '0' : '1';
    $('.measurement-rail').style.opacity = id === 'end' ? '.35' : '1';
    $('#sceneVerb').textContent = verb;
  }

  function renderPremise(time, looping = false) {
    showScene('premise', 'PROVE IT');
    const local = looping ? between(time, 20.62, 21) : between(time, 0, 2);
    const reveal = smooth(looping ? local : between(local, .05, .55));
    $('.premise h1').style.transform = `scale(${.94 + reveal * .06})`;
    $('.premise h1').style.opacity = String(.35 + reveal * .65);
    $('.premise strong').style.opacity = String(smooth(between(local, .35, .85)));
    const angle = local * Math.PI * 2;
    $$('.premise-orb i').forEach((dot, index) => {
      const radius = 1.2 + index * .75;
      dot.style.transform = `translate(${Math.cos(angle + index * 2.1) * radius}cqw,${Math.sin(angle + index * 2.1) * radius}cqw)`;
    });
    $('#railTitle').textContent = 'PROVE IT';
    $('#railPin').style.display = 'none';
  }

  function renderMeasure(time) {
    showScene('measure', 'MEASURE');
    if (!evidence) return;
    const local = between(time, 2, 9);
    const finalPass = local >= .53;
    const index = finalPass ? 1 : 0;
    const scan = 18 + (Math.sin(local * Math.PI * 2.4) * .5 + .5) * 64;
    setBackground('.build-art', evidence.assets.builds[index]);
    setBackground('.heatmap-art', evidence.assets.heatmaps[index]);
    $('.build-art').style.clipPath = `inset(0 0 0 ${scan}%)`;
    $('.heatmap-art').style.clipPath = `inset(0 ${100 - scan}% 0 0)`;
    $('.heatmap-art').style.opacity = String(finalPass ? .5 : .92);
    $('#scanLine').style.left = `${scan}%`;
    $('#checkpointLabel').textContent = finalPass ? 'AGENT · VERIFIED PASS' : 'AGENT · FIRST PASS';
    $('#scoreBefore').textContent = `${Number(evidence.scores[0]).toFixed(2)}%`;
    $('#scoreNow').textContent = `${Number(evidence.scores[index]).toFixed(2)}%`;
    $('#scoreNow').style.transform = finalPass ? 'scale(1.08)' : 'scale(1)';
    $('#scoreNow').style.color = finalPass ? 'var(--mint)' : 'var(--paper)';
    $('#railTitle').textContent = finalPass ? 'VERIFIED PASS' : 'PIXEL DIFF';
    $('#railPin').style.display = 'none';
  }

  function motionStep(local) {
    if (local < .18) return 0;
    if (local < .46) return 1;
    if (local < .69) return 2;
    return 3;
  }

  function renderMotion(time) {
    showScene('motion', 'PIN MOTION');
    if (!evidence) return;
    const local = between(time, 9, 15);
    const index = motionStep(local);
    const progress = [0, 50, 80, 50][index];
    setBackground('.reference-motion', evidence.assets.motionReferenceFrames[index]);
    setBackground('.build-motion', evidence.assets.motionBuildFrames[index]);
    $$('.frame-readout').forEach((label) => { label.textContent = `${progress}%`; });
    $$('#motionSequence span').forEach((step, stepIndex) => step.classList.toggle('active', stepIndex === index));
    $('#scrubFill').style.width = `${progress}%`;
    $('#scrubPin').style.left = `${progress}%`;
    $('#repeatProof').textContent = index === 3 ? '✓ SAME 50% FRAME · EXACT REPEAT' : 'SCRUBBING BOTH PAGES TOGETHER';
    $('#railTitle').textContent = index === 3 ? 'EXACT REPEAT' : 'ANIMATION TIME';
    $('#railPin').style.display = 'block';
    $('#railPin').style.left = `${progress}%`;
  }

  function renderHandoff(time) {
    showScene('handoff', 'HAND OFF');
    if (!evidence) return;
    const local = between(time, 15, 19);
    const focus = smooth(between(local, .22, .62));
    const pulse = between(local, .15, .48);
    setBackground('.handoff-image', evidence.assets.handoff);
    $('.handoff-image').style.filter = `saturate(${.65 + focus * .35}) brightness(${.62 + focus * .38})`;
    $('#handoffBrowser').style.transform = `scale(${.96 + focus * .04})`;
    $('#browserTitle').textContent = focus > .5 ? 'YOUR WINDOW · SAME BROWSER' : 'AGENT’S BROWSER · OFFSCREEN';
    $('#liveBadge').textContent = focus > .5 ? 'LIVE' : 'INVISIBLE';
    $('#liveBadge').style.borderColor = focus > .5 ? 'var(--mint)' : 'var(--amber)';
    $('#liveBadge').style.color = focus > .5 ? 'var(--mint)' : 'var(--amber)';
    $('#focusPulse').style.opacity = String(Math.sin(pulse * Math.PI) * (pulse > 0 && pulse < 1 ? 1 : 0));
    $('#focusPulse').style.transform = `translate(-50%,-50%) scale(${.75 + pulse * .5})`;
    $('.typed-callout').style.opacity = String(.35 + focus * .65);
    $('.preserved-row').style.opacity = String(smooth(between(local, .55, .9)));
    $('#railTitle').textContent = focus > .5 ? 'STATE PRESERVED' : 'RUNNING OFFSCREEN';
    $('#railPin').style.display = 'none';
  }

  function renderEnd(time) {
    showScene('end', 'HWATU');
    const local = between(time, 19, 20.62);
    $('.end').style.opacity = String(smooth(between(local, 0, .25)) * (1 - smooth(between(local, .78, 1))));
    $('.end-mark').style.transform = `scale(${.75 + smooth(local) * .25})`;
    $('#railTitle').textContent = 'PROOF, NOT PROMISES';
    $('#railPin').style.display = 'none';
  }

  function render(rawTime) {
    const time = clamp(rawTime, 0, DURATION);
    $('#stage').style.setProperty('--drift', `${time * 2}cqw`);
    if (time < 2) renderPremise(time);
    else if (time < 9) renderMeasure(time);
    else if (time < 15) renderMotion(time);
    else if (time < 19) renderHandoff(time);
    else if (time < 20.62) renderEnd(time);
    else renderPremise(time, true);
    $('#railFill').style.width = `${(time / DURATION) * 100}%`;
    $('#railValue').textContent = `${time.toFixed(1).padStart(4, '0')} / ${DURATION.toFixed(1)}`;
    document.documentElement.dataset.time = time.toFixed(3);
  }

  const started = performance.now();
  function tick(now) {
    const offset = Number(params.get('t')) || 0;
    render((offset + (now - started) / 1000) % DURATION);
    requestAnimationFrame(tick);
  }

  loadEvidence().then(() => {
    render(fixedTime ?? 0);
    if (fixedTime === null) requestAnimationFrame(tick);
  });
})();
