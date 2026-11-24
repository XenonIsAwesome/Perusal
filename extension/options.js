const defaults = { "github.com": "github", "gitlab.com": "gitlab" };
async function render() {
  const { sites } = await chrome.storage.sync.get({ sites: defaults });
  const ul = document.getElementById("list"); ul.textContent = "";
  for (const [h, k] of Object.entries(sites)) { const li = document.createElement("li"); li.textContent = h + " (" + k + ")"; ul.appendChild(li); }
}
document.getElementById("f").addEventListener("submit", async (e) => {
  e.preventDefault();
  const host = document.getElementById("host").value.trim().toLowerCase();
  const granted = await chrome.permissions.request({ origins: ["https://" + host + "/*"] });
  if (!granted) return;
  const { sites } = await chrome.storage.sync.get({ sites: defaults });
  sites[host] = document.getElementById("kind").value;
  await chrome.storage.sync.set({ sites }); render();
});
render();
