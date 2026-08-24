const assert = require("node:assert/strict");
const test = require("node:test");

const { validateDownloadUrl } = require("../scripts/postinstall.js");

test("accepts only HTTPS GitHub release hosts", () => {
  assert.equal(validateDownloadUrl("https://github.com/owner/repo").hostname, "github.com");
  assert.equal(
    validateDownloadUrl("https://release-assets.githubusercontent.com/file").hostname,
    "release-assets.githubusercontent.com"
  );
  assert.throws(() => validateDownloadUrl("http://github.com/owner/repo"), /untrusted/);
  assert.throws(() => validateDownloadUrl("https://github.com.example.org/file"), /untrusted/);
});
