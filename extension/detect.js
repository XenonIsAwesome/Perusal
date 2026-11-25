// Detects MR/PR pages by path and builds the perusal:// link. Shared by the content script and tests.
function detect(urlStr, sites) {
  let u; try { u = new URL(urlStr); } catch { return null; }
  if (u.protocol !== "https:") return null;
  const kind = sites[u.hostname];
  if (!kind) return null;
  const p = u.pathname.replace(/\/+$/, "");
  const ok = kind === "github" ? /^\/[^/]+\/[^/]+\/pull\/\d+(\/.*)?$/.test(p) : /^\/.+\/-\/merge_requests\/\d+(\/.*)?$/.test(p);
  return ok ? "perusal://open?url=" + encodeURIComponent(u.origin + p.replace(/\/(files|commits|diffs).*$/, "")) : null;
}
if (typeof module !== "undefined") module.exports = { detect };
