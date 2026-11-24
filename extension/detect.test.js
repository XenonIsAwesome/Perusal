const assert = require("assert"); const { detect } = require("./detect.js");
const sites = { "github.com": "github", "gitlab.com": "gitlab", "git.corp": "gitlab" };
assert.strictEqual(detect("https://github.com/o/r/pull/12/files", sites), "perusal://open?url=" + encodeURIComponent("https://github.com/o/r/pull/12"));
assert.strictEqual(detect("https://git.corp/a/b/-/merge_requests/9/diffs", sites), "perusal://open?url=" + encodeURIComponent("https://git.corp/a/b/-/merge_requests/9"));
assert.strictEqual(detect("https://github.com/o/r", sites), null);
assert.strictEqual(detect("http://github.com/o/r/pull/1", sites), null);
assert.strictEqual(detect("https://evil.com/o/r/pull/1", sites), null);
assert.strictEqual(detect("https://gitlab.com/o/r/pull/1", sites), null);
console.log("extension tests ok");
