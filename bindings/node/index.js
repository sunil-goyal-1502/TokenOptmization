const { existsSync } = require("fs");
const { join } = require("path");

let binding;
const local = join(__dirname, "tokenopt.linux-x64-gnu.node");
if (existsSync(local)) {
  binding = require(local);
} else {
  try {
    binding = require("./tokenopt.node");
  } catch {
    binding = null;
  }
}

function ensure() {
  if (!binding) {
    throw new Error(
      "TokenOpt native addon not built. Run: cd bindings/node && npm install && npm run build",
    );
  }
  return binding;
}

module.exports = {
  nativeAvailable: () => Boolean(binding),
  compileJson: (messagesJson, optionsJson = "{}") =>
    ensure().compileJson(messagesJson, optionsJson),
  compareJson: (messagesJson, optionsJson = "{}") =>
    ensure().compareJson(messagesJson, optionsJson),
  analyzeJson: (messagesJson) => ensure().analyzeJson(messagesJson),
};
