(async () => {
  const { sites } = await chrome.storage.sync.get({ sites: { "github.com": "github", "gitlab.com": "gitlab" } });
  const link = detect(location.href, sites);
  if (!link || document.getElementById("perusal-open")) return;
  const a = document.createElement("a");
  a.id = "perusal-open"; a.textContent = "Open in Perusal"; a.href = link;
  a.style.cssText = "position:fixed;right:16px;bottom:16px;z-index:99999;padding:8px 12px;background:#0969da;color:#fff;border-radius:6px;font:14px sans-serif;text-decoration:none";
  document.body.appendChild(a);
})();
