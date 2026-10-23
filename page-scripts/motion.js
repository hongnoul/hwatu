// Declared motion inventory for `motion`: a function body run by eval.
// getAnimations() for running CSS/WAAPI animations and transitions, plus a
// CSSOM walk for @keyframes and idle declared transitions/animations.
const MAX = 200;
const sel = (el) => {
  if (!el || !el.tagName) return null;
  let s = el.tagName.toLowerCase();
  if (el.id) return s + '#' + el.id;
  if (el.classList && el.classList.length) s += '.' + [...el.classList].slice(0, 3).join('.');
  return s;
};
const animations = document.getAnimations().slice(0, MAX).map(a => {
  const out = { state: a.playState };
  if (a.animationName) out.name = a.animationName;           // CSSAnimation
  if (a.transitionProperty) out.property = a.transitionProperty; // CSSTransition
  const ef = a.effect;
  if (ef) {
    const t = ef.getTiming();
    out.duration_ms = t.duration;
    out.delay_ms = t.delay;
    out.easing = t.easing;
    out.iterations = t.iterations;
    out.direction = t.direction;
    out.fill = t.fill;
    if (ef.target) out.target = sel(ef.target);
    try {
      out.keyframes = ef.getKeyframes().map(k => {
        const kf = {};
        for (const [key, v] of Object.entries(k)) {
          if (key === 'composite' || key === 'computedOffset') continue;
          kf[key] = v;
        }
        return kf;
      });
    } catch (e) { /* cross-origin effect */ }
  }
  out.current_time_ms = a.currentTime;
  return out;
});
// CSSOM: @keyframes rules + elements with declared transitions that
// are idle right now (they only show up in getAnimations mid-flight).
const keyframes = {};
const kfSeen = new Set();
for (const sheet of document.styleSheets) {
  let rules;
  try { rules = sheet.cssRules; } catch (e) { continue; } // cross-origin
  const walk = (list) => {
    for (const r of list) {
      if (r.type === CSSRule.KEYFRAMES_RULE && !kfSeen.has(r.name)) {
        kfSeen.add(r.name);
        keyframes[r.name] = [...r.cssRules].map(k => ({ offset: k.keyText, style: k.style.cssText }));
      } else if (r.cssRules) {
        walk(r.cssRules);
      }
    }
  };
  walk(rules);
}
const declared = [];
const els = document.querySelectorAll('*');
for (const el of els) {
  if (declared.length >= MAX) break;
  const st = getComputedStyle(el);
  const tp = st.transitionProperty;
  const td = st.transitionDuration;
  const hasTransition = tp && tp !== 'all' || (td && td.split(',').some(d => parseFloat(d) > 0));
  const hasAnimation = st.animationName && st.animationName !== 'none';
  if (!hasTransition && !hasAnimation) continue;
  const d = { target: sel(el) };
  if (hasTransition) {
    d.transition = {
      property: tp, duration: td,
      timing: st.transitionTimingFunction, delay: st.transitionDelay,
    };
  }
  if (hasAnimation) {
    d.animation = {
      name: st.animationName, duration: st.animationDuration,
      timing: st.animationTimingFunction, delay: st.animationDelay,
      iterations: st.animationIterationCount, direction: st.animationDirection,
      fill: st.animationFillMode, state: st.animationPlayState,
    };
  }
  declared.push(d);
}
return { animations, keyframes, declared };
