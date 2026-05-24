/**
 * Loader for @tokenopt/native — uses napi-rs artifact when built.
 */
const { existsSync, readdirSync } = require("fs");
const { join } = require("path");

function loadBinding() {
  const dir = __dirname;
  const candidates = readdirSync(dir).filter((f) => f.endsWith(".node"));
  for (const name of candidates) {
    const path = join(dir, name);
    if (existsSync(path)) {
      return require(path);
    }
  }
  if (existsSync(join(dir, "tokenopt.node"))) {
    return require(join(dir, "tokenopt.node"));
  }
  return null;
}

const binding = loadBinding();

module.exports = {
  nativeAvailable: () => Boolean(binding),
  compileJson: (messagesJson, optionsJson = "{}") => {
    if (!binding) {
      return Promise.reject(
        new Error("Run: cd bindings/node && npm install && npm run build"),
      );
    }
    return binding.compileJson(messagesJson, optionsJson);
  },
  compareJson: (messagesJson, optionsJson = "{}") => {
    if (!binding) {
      return Promise.reject(new Error("Native addon not built"));
    }
    return binding.compareJson(messagesJson, optionsJson);
  },
  analyzeJson: (messagesJson) => {
    if (!binding) {
      throw new Error("Native addon not built");
    }
    return binding.analyzeJson(messagesJson);
  },
};
