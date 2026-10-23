// `auth-context`: cookie and storage inventory (names only, never
// values) plus the login-wall probe. The caller substitutes the login-wall
// placeholder below with page-scripts/login-wall.js.
const out = { url: location.href, origin: location.origin };
try {
  const names = document.cookie ? document.cookie.split('; ').map(c => c.split('=')[0]).filter(Boolean) : [];
  out.cookie_names = names.slice(0, 50);
  out.cookie_count = names.length;
} catch (_) { out.cookie_count = 0; out.cookie_names = []; }
if (window.cookieStore && cookieStore.getAll) {
  try {
    const all = await cookieStore.getAll();
    out.cookie_count = all.length;
    out.cookie_names = all.map(c => c.name).slice(0, 50);
    const now = Date.now();
    out.persistent_cookies = all.filter(c => c.expires && c.expires > now).length;
    out.session_cookies = all.filter(c => !c.expires).length;
  } catch (_) {}
}
try {
  out.local_storage_keys = Object.keys(localStorage).slice(0, 50);
  out.local_storage_count = localStorage.length;
} catch (_) { out.local_storage_count = 0; }
try { out.session_storage_count = sessionStorage.length; } catch (_) { out.session_storage_count = 0; }
out.likely_authenticated = out.cookie_count > 0
  && (out.cookie_names || []).some(n => /sess|token|auth|sid|login|jwt|csrf/i.test(n));
out.login_wall = __LOGIN_WALL__;
return out;
